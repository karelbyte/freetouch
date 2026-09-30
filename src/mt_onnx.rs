use anyhow::{Context, bail};
use ort::session::Session;
use ort::value::Tensor;
use std::time::Instant;
use tokenizers::Tokenizer;

fn oe<E: std::fmt::Debug>(e: E) -> anyhow::Error {
    anyhow::anyhow!("ort: {e:?}")
}

type Past = Vec<(String, Vec<i64>, Vec<f32>)>;

/// Traductor MarianMT real (Helsinki-NLP vía Xenova, ONNX cuantizado).
/// Greedy (beam 1) con caché KV: `dec0` en el primer paso,
/// `dec_past` después (solo refresca el past del decoder).
/// Greedy pierde poco frente a beam en frases cortas y hace ~4x menos
/// pasadas ONNX por token: clave para latencia en vivo.
pub struct MarianTranslator {
    encoder: Session,
    dec0: Session,
    decpast: Session,
    tokenizer: Tokenizer,
    enc_input_ids: String,
    enc_attn_mask: String,
    d0_input_ids: String,
    d0_enc_hidden: String,
    d0_enc_mask: String,
    dp_input_ids: String,
    dp_enc_mask: String,
    dp_out_names: Vec<String>,
    decoder_start_id: i64,
    eos_id: i64,
    max_tokens: usize,
    repetition_penalty: f32,
}

/// top-k con penalización por repetición y bloqueo de trigramas.
fn top_k(row: &[f32], vocab: usize, k: usize, ids: &[i64], pen: f32) -> Vec<(i64, f32)> {
    let mut banned = vec![false; vocab];
    if ids.len() >= 2 {
        let (a, b) = (ids[ids.len() - 2], ids[ids.len() - 1]);
        for w in ids.windows(3) {
            if w[0] == a && w[1] == b && (w[2] as usize) < vocab {
                banned[w[2] as usize] = true;
            }
        }
    }
    let mut top: Vec<(i64, f32)> = Vec::with_capacity(k);
    for (i, &v) in row.iter().enumerate().take(vocab) {
        if banned[i] {
            continue;
        }
        let already = ids.iter().any(|&t| t as usize == i);
        let s = if already {
            if v > 0.0 { v / pen } else { v * pen }
        } else {
            v
        };
        if top.len() < k {
            top.push((i as i64, s));
            if top.len() == k {
                top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            }
        } else if s > top[k - 1].1 {
            top[k - 1] = (i as i64, s);
            top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        }
    }
    top
}

impl MarianTranslator {
    pub fn new(dir: &std::path::Path, threads: usize) -> anyhow::Result<Self> {
        for f in ["encoder.onnx", "dec0.onnx", "dec_past.onnx", "tokenizer.json", "generation_config.json"] {
            if !dir.join(f).exists() {
                bail!("falta {}", dir.join(f).display());
            }
        }
        let mut sessions = Vec::new();
        for f in ["encoder.onnx", "dec0.onnx", "dec_past.onnx"] {
            sessions.push(
                Session::builder()
                    .map_err(oe)?
                    .with_intra_threads(threads)
                    .map_err(oe)?
                    .commit_from_file(&dir.join(f))
                    .map_err(oe)?,
            );
        }
        let mut sessions = sessions.into_iter();
        let encoder = sessions.next().unwrap();
        let dec0 = sessions.next().unwrap();
        let decpast = sessions.next().unwrap();
        let tokenizer = Tokenizer::from_file(&dir.join("tokenizer.json"))
            .map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;

        let names = |s: &Session| {
            s.inputs().iter().map(|o| o.name().to_string()).collect::<Vec<_>>()
        };
        let pick = |names: &[String], needle: &str| -> anyhow::Result<String> {
            names
                .iter()
                .find(|n| n.contains(needle))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("input '{needle}' no hallado en {names:?}"))
        };
        let enc_names = names(&encoder);
        let d0_names = names(&dec0);
        let dp_names = names(&decpast);
        let enc_input_ids = pick(&enc_names, "input_ids")?;
        let enc_attn_mask =
            pick(&enc_names, "attention_mask").unwrap_or_else(|_| enc_input_ids.clone());
        let d0_input_ids = pick(&d0_names, "input_ids")?;
        let d0_enc_hidden = pick(&d0_names, "encoder_hidden")?;
        let d0_enc_mask = pick(&d0_names, "encoder_attention")
            .unwrap_or_else(|_| d0_enc_hidden.clone());
        let dp_input_ids = pick(&dp_names, "input_ids")?;
        let dp_enc_mask = pick(&dp_names, "encoder_attention")
            .or_else(|_| pick(&dp_names, "attention_mask"))
            .unwrap_or_else(|_| dp_input_ids.clone());
        let dp_out_names: Vec<String> =
            decpast.outputs().iter().map(|o| o.name().to_string()).collect();

        let gen_cfg: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("generation_config.json"))
                .context("leyendo generation_config")?,
        )?;
        let decoder_start_id = gen_cfg
            .get("decoder_start_token_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(65000);
        let eos_id = gen_cfg.get("eos_token_id").and_then(|v| v.as_i64()).unwrap_or(0);

        Ok(Self {
            encoder,
            dec0,
            decpast,
            tokenizer,
            enc_input_ids,
            enc_attn_mask,
            d0_input_ids,
            d0_enc_hidden,
            d0_enc_mask,
            dp_input_ids,
            dp_enc_mask,
            dp_out_names,
            decoder_start_id,
            eos_id,
            max_tokens: 64,
            repetition_penalty: 1.2,
        })
    }

    /// Extrae el past de los outputs (outputs[0] = logits).
    /// `present.X` -> `past_key_values.X`.
    fn extract_past(
        dec_out: &ort::session::SessionOutputs<'_>,
        out_names: &[String],
    ) -> anyhow::Result<Past> {
        let mut past = Vec::new();
        for (i, name) in out_names.iter().enumerate().skip(1) {
            if let Some(key) = name.strip_prefix("present.") {
                let in_name = format!("past_key_values.{key}");
                let (shape, data) = dec_out[i].try_extract_tensor::<f32>().map_err(oe)?;
                past.push((in_name, shape.to_vec(), data.to_vec()));
            }
        }
        past.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(past)
    }

    pub fn translate(&mut self, text: &str) -> anyhow::Result<(String, u128)> {
        let t0 = Instant::now();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            bail!("texto vacío");
        }
        let enc = self
            .tokenizer
            .encode(trimmed, true)
            .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
        if ids.is_empty() {
            bail!("sin tokens");
        }
        let n = ids.len();
        let (hidden_dim, enc_hidden_flat) = {
            let enc_inputs = ort::inputs! {
                self.enc_input_ids.clone() => Tensor::from_array((vec![1, n as i64], ids.clone())).map_err(oe)?,
                self.enc_attn_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
            };
            let enc_out = self.encoder.run(enc_inputs).map_err(oe)?;
            let (_, enc_data) = enc_out[0].try_extract_tensor::<f32>().map_err(oe)?;
            (enc_data.len() / n, enc_data.to_vec())
        };

        const BEAMS: usize = 1;
        let start_id = self.decoder_start_id;
        let eos = self.eos_id;
        let pen = self.repetition_penalty;
        let max_tokens = self.max_tokens;
        let out_names = self.dp_out_names.clone();

        // Paso 0 con dec0 (sin past): logits + past completo (24).
        // Los outputs de dec0 tienen los mismos nombres present.*.
        let d0_out_names: Vec<String> =
            self.dec0.outputs().iter().map(|o| o.name().to_string()).collect();
        let (vocab, first_row, past0) = {
            let dec_inputs = ort::inputs! {
                self.d0_input_ids.clone() => Tensor::from_array((vec![1, 1], vec![start_id])).map_err(oe)?,
                self.d0_enc_hidden.clone() => Tensor::from_array((vec![1, n as i64, hidden_dim as i64], enc_hidden_flat.clone())).map_err(oe)?,
                self.d0_enc_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
            };
            let dec_out = self.dec0.run(dec_inputs).map_err(oe)?;
            let (shape, logits) = dec_out[0].try_extract_tensor::<f32>().map_err(oe)?;
            let vocab = shape[2] as usize;
            let row = logits[0..vocab].to_vec();
            let past0 = Self::extract_past(&dec_out, &d0_out_names).map_err(oe)?;
            (vocab, row, past0)
        };
        // Parte el past0 en encoder (constante) y decoder (por haz).
        let mut enc_past0: Past = Vec::new();
        let mut dec_past0: Past = Vec::new();
        for p in past0 {
            if p.0.contains(".encoder.") {
                enc_past0.push(p);
            } else {
                dec_past0.push(p);
            }
        }

        // Haces iniciales: top_k del paso 0, todos con el mismo dec_past0.
        let mut hyps: Vec<(Vec<i64>, f32, Past)> = top_k(&first_row, vocab, BEAMS, &[], 1.3)
            .into_iter()
            .map(|(tok, s)| (vec![start_id, tok], s, dec_past0.clone()))
            .collect();
        let mut finished: Vec<(Vec<i64>, f32)> = Vec::new();
        hyps.retain(|(ids, s, _)| {
            if *ids.last().unwrap() == eos {
                finished.push((ids.clone(), *s));
                false
            } else {
                true
            }
        });
        for _ in 1..max_tokens {
            if hyps.is_empty() || finished.len() >= BEAMS {
                break;
            }
            let mut cands: Vec<(Vec<i64>, f32, Past)> = Vec::new();
            for (ids, score, dec_past) in &hyps {
                let last = *ids.last().unwrap();
                // Past completo: encoder constante + decoder de este haz.
                let mut full_past = enc_past0.clone();
                full_past.extend(dec_past.iter().cloned());
                full_past.sort_by(|a, b| a.0.cmp(&b.0));
                let (row, new_full) = {
                    let mut dec_inputs = ort::inputs! {
                        self.dp_input_ids.clone() => Tensor::from_array((vec![1, 1], vec![last])).map_err(oe)?,
                        self.dp_enc_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
                    };
                    for (pname, pshape, pdata) in &full_past {
                        dec_inputs.push((
                            pname.clone().into(),
                            Tensor::from_array((pshape.clone(), pdata.clone()))
                                .map_err(oe)?
                                .into(),
                        ));
                    }
                    let dec_out = self.decpast.run(dec_inputs).map_err(oe)?;
                    let (_, logits) = dec_out[0].try_extract_tensor::<f32>().map_err(oe)?;
                    let row = logits[0..vocab].to_vec();
                    let new_full = Self::extract_past(&dec_out, &out_names).map_err(oe)?;
                    (row, new_full)
                };
                // Solo el past decoder cambia; el encoder se reinyecta igual.
                let new_dec_past: Past = new_full
                    .into_iter()
                    .filter(|(nm, _, _)| nm.contains(".decoder."))
                    .collect();
                for (tok, s) in top_k(&row, vocab, BEAMS, ids, pen) {
                    let mut next = ids.clone();
                    next.push(tok);
                    // Suma cruda; la normalización por longitud solo se usa
                    // al comparar (si no, premia frases infinitas).
                    cands.push((next, score + s, new_dec_past.clone()));
                }
            }
            // Ordena por suma/len^0.6 (mayor = mejor).
            cands.sort_by(|a, b| {
                let na = a.1 / ((a.0.len() as f32) / 6.0).powf(0.6);
                let nb = b.1 / ((b.0.len() as f32) / 6.0).powf(0.6);
                nb.partial_cmp(&na).unwrap()
            });
            let mut next_hyps = Vec::with_capacity(BEAMS);
            for (ids, score, dec_past) in cands {
                if *ids.last().unwrap() == eos {
                    finished.push((ids, score));
                    if finished.len() >= BEAMS {
                        break;
                    }
                } else if next_hyps.len() < BEAMS {
                    next_hyps.push((ids, score, dec_past));
                }
                if next_hyps.len() >= BEAMS && finished.len() >= BEAMS {
                    break;
                }
            }
            hyps = next_hyps;
        }
        for (ids, score, _) in hyps {
            finished.push((ids, score));
        }
        finished.sort_by(|a, b| {
            let na = a.1 / ((a.0.len() as f32) / 6.0).powf(0.6);
            let nb = b.1 / ((b.0.len() as f32) / 6.0).powf(0.6);
            nb.partial_cmp(&na).unwrap()
        });
        let best_ids = finished.into_iter().next().map(|(ids, _)| ids).unwrap_or_default();
        let out_ids: Vec<u32> = best_ids
            .into_iter()
            .skip(1)
            .map(|x| x as u32)
            .filter(|&x| x as i64 != eos)
            .collect();
        let text = self
            .tokenizer
            .decode(&out_ids, true)
            .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
        Ok((text.trim().to_owned(), t0.elapsed().as_millis()))
    }

    /// Codifica texto a ids (para prefijos forzados).
    pub fn encode_ids(&self, text: &str, add_special: bool) -> anyhow::Result<Vec<i64>> {
        let enc = self
            .tokenizer
            .encode(text, add_special)
            .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        Ok(enc.get_ids().iter().map(|&x| x as i64).collect())
    }

    /// Greedy con prefijo forzado: ejecuta el decoder sobre cada token del
    /// prefijo (para el estado KV) y luego argmax. Devuelve SOLO el texto
    /// nuevo (sin el prefijo). Así la salida es extensión monótona de lo
    /// ya entregado: clave del streaming sin mezclas.
    pub fn translate_with_prefix(
        &mut self,
        text: &str,
        prefix_words: &[String],
    ) -> anyhow::Result<(String, u128)> {
        let t0 = Instant::now();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            bail!("texto vacío");
        }
        let enc = self
            .tokenizer
            .encode(trimmed, true)
            .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
        if ids.is_empty() {
            bail!("sin tokens");
        }
        let mut prefix: Vec<i64> = if prefix_words.is_empty() {
            Vec::new()
        } else {
            self.encode_ids(&prefix_words.join(" "), false)?
        };
        prefix.retain(|&t| t != self.eos_id);
        // Limita el prefijo a la cola (cota el coste por llamada).
        if prefix.len() > 12 {
            prefix.drain(0..prefix.len() - 12);
        }
        let n = ids.len();
        let (hidden_dim, enc_hidden_flat) = {
            let enc_inputs = ort::inputs! {
                self.enc_input_ids.clone() => Tensor::from_array((vec![1, n as i64], ids.clone())).map_err(oe)?,
                self.enc_attn_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
            };
            let enc_out = self.encoder.run(enc_inputs).map_err(oe)?;
            let (_, enc_data) = enc_out[0].try_extract_tensor::<f32>().map_err(oe)?;
            (enc_data.len() / n, enc_data.to_vec())
        };

        let start_id = self.decoder_start_id;
        let eos = self.eos_id;
        let pen = self.repetition_penalty;
        let max_tokens = self.max_tokens;
        let out_names = self.dp_out_names.clone();
        let d0_out_names: Vec<String> =
            self.dec0.outputs().iter().map(|o| o.name().to_string()).collect();
        let (first_row, past0) = {
            let dec_inputs = ort::inputs! {
                self.d0_input_ids.clone() => Tensor::from_array((vec![1, 1], vec![start_id])).map_err(oe)?,
                self.d0_enc_hidden.clone() => Tensor::from_array((vec![1, n as i64, hidden_dim as i64], enc_hidden_flat.clone())).map_err(oe)?,
                self.d0_enc_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
            };
            let dec_out = self.dec0.run(dec_inputs).map_err(oe)?;
            let (shape, logits) = dec_out[0].try_extract_tensor::<f32>().map_err(oe)?;
            let row = logits[0..shape[2] as usize].to_vec();
            let past0 = Self::extract_past(&dec_out, &d0_out_names).map_err(oe)?;
            (row, past0)
        };
        let mut enc_past0: Past = Vec::new();
        let mut dec_past: Past = Vec::new();
        for p in past0 {
            if p.0.contains(".encoder.") {
                enc_past0.push(p);
            } else {
                dec_past.push(p);
            }
        }
        // Fila pendiente para la proxima posicion; cada dec_step alimenta
        // exactamente un token no consumido (sin dobles).
        let mut row = first_row;
        let mut ids_out = vec![start_id];
        for &p in &prefix {
            let (r, np) = self.dec_step(p, n, &enc_past0, dec_past, &out_names)?;
            row = r;
            dec_past = np;
            ids_out.push(p);
        }
        // Greedy con penalización y bloqueo de trigramas.
        let mut new_tokens: Vec<i64> = Vec::new();
        let mut fresh = 0usize;
        while fresh < max_tokens {
            let tok = Self::greedy_pick(&row, &ids_out, pen);
            if tok == eos {
                break;
            }
            ids_out.push(tok);
            new_tokens.push(tok);
            fresh += 1;
            let (r, np) = self.dec_step(tok, n, &enc_past0, dec_past, &out_names)?;
            row = r;
            dec_past = np;
        }
        let out_ids: Vec<u32> = new_tokens.into_iter().map(|x| x as u32).collect();
        let text = self
            .tokenizer
            .decode(&out_ids, true)
            .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
        Ok((text.trim().to_owned(), t0.elapsed().as_millis()))
    }

/// Argmax con penalización por repetición y bloqueo de trigramas.
fn greedy_pick(row: &[f32], ids: &[i64], pen: f32) -> i64 {
    let vocab = row.len();
    let mut banned = vec![false; vocab];
    if ids.len() >= 2 {
        let (a, b) = (ids[ids.len() - 2], ids[ids.len() - 1]);
        for w in ids.windows(3) {
            if w[0] == a && w[1] == b && (w[2] as usize) < vocab {
                banned[w[2] as usize] = true;
            }
        }
    }
    let (mut best, mut best_v) = (0i64, f32::NEG_INFINITY);
    for (i, &v) in row.iter().enumerate() {
        if banned[i] {
            continue;
        }
        let already = ids.iter().any(|&t| t as usize == i);
        let s = if already {
            if v > 0.0 { v / pen } else { v * pen }
        } else {
            v
        };
        if s > best_v {
            best_v = s;
            best = i as i64;
        }
    }
    best
}

    fn dec_step(
        &mut self,
        last: i64,
        n: usize,
        enc_past: &Past,
        dec_past: Past,
        out_names: &[String],
    ) -> anyhow::Result<(Vec<f32>, Past)> {
        let mut full_past = enc_past.to_vec();
        full_past.extend(dec_past.into_iter());
        full_past.sort_by(|a, b| a.0.cmp(&b.0));
        let mut dec_inputs = ort::inputs! {
            self.dp_input_ids.clone() => Tensor::from_array((vec![1, 1], vec![last])).map_err(oe)?,
            self.dp_enc_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
        };
        for (pname, pshape, pdata) in &full_past {
            dec_inputs.push((
                pname.clone().into(),
                Tensor::from_array((pshape.clone(), pdata.clone()))
                    .map_err(oe)?
                    .into(),
            ));
        }
        let dec_out = self.decpast.run(dec_inputs).map_err(oe)?;
        let (_, logits) = dec_out[0].try_extract_tensor::<f32>().map_err(oe)?;
        let row = logits.to_vec();
        let new_full = Self::extract_past(&dec_out, out_names).map_err(oe)?;
        let new_dec_past: Past = new_full
            .into_iter()
            .filter(|(nm, _, _)| nm.contains(".decoder."))
            .collect();
        Ok((row, new_dec_past))
    }

    /// Diagnóstico: greedy con recomputación total vía dec0 (sin caché).
    #[allow(dead_code)]
    pub fn translate_full_recompute(&mut self, text: &str) -> anyhow::Result<(String, u128)> {
        let t0 = Instant::now();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            bail!("texto vacío");
        }
        let enc = self
            .tokenizer
            .encode(trimmed, true)
            .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
        let n = ids.len();
        let (hidden_dim, enc_hidden_flat) = {
            let enc_inputs = ort::inputs! {
                self.enc_input_ids.clone() => Tensor::from_array((vec![1, n as i64], ids.clone())).map_err(oe)?,
                self.enc_attn_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
            };
            let enc_out = self.encoder.run(enc_inputs).map_err(oe)?;
            let (_, enc_data) = enc_out[0].try_extract_tensor::<f32>().map_err(oe)?;
            (enc_data.len() / n, enc_data.to_vec())
        };
        let mut dec_ids = vec![self.decoder_start_id];
        for _ in 0..30 {
            let m = dec_ids.len();
            let dec_inputs = ort::inputs! {
                self.d0_input_ids.clone() => Tensor::from_array((vec![1, m as i64], dec_ids.clone())).map_err(oe)?,
                self.d0_enc_hidden.clone() => Tensor::from_array((vec![1, n as i64, hidden_dim as i64], enc_hidden_flat.clone())).map_err(oe)?,
                self.d0_enc_mask.clone() => Tensor::from_array((vec![1, n as i64], vec![1i64; n])).map_err(oe)?
            };
            let dec_out = self.dec0.run(dec_inputs).map_err(oe)?;
            let (shape, logits) = dec_out[0].try_extract_tensor::<f32>().map_err(oe)?;
            let vocab = shape[2] as usize;
            let row = &logits[(m - 1) * vocab..m * vocab];
            let (mut best, mut best_v) = (0i64, f32::NEG_INFINITY);
            for (i, &v) in row.iter().enumerate() {
                if v > best_v {
                    best_v = v;
                    best = i as i64;
                }
            }
            if best == self.eos_id {
                break;
            }
            dec_ids.push(best);
        }
        let out_ids: Vec<u32> = dec_ids.into_iter().skip(1).map(|x| x as u32).collect();
        let text = self.tokenizer.decode(&out_ids, true).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok((text.trim().to_owned(), t0.elapsed().as_millis()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diag_full_recompute() {
        let dir = std::path::Path::new("models/mt/en-es");
        if !dir.join("encoder.onnx").exists() {
            return;
        }
        let mut t = MarianTranslator::new(dir, 4).unwrap();
        let (text, ms) = t.translate_full_recompute("hello, how are you").unwrap();
        eprintln!("FULL-RECOMPUTE '{text}' ({ms}ms)");
    }

    #[test]
    fn en_hello_to_hola() {
        let dir = std::path::Path::new("models/mt/en-es");
        if !dir.join("encoder.onnx").exists() {
            eprintln!("omitido: sin modelos MT");
            return;
        }
        let mut t = MarianTranslator::new(dir, 4).unwrap();
        let (text, ms) = t.translate("hello, how are you").unwrap();
        eprintln!("EN->ES '{text}' ({ms}ms)");
        assert!(
            text.to_lowercase().contains("cómo estás") || text.to_lowercase().contains("hola"),
            "obtenido: {text}"
        );
    }

    #[test]
    fn es_gracias_to_thank_you() {
        let dir = std::path::Path::new("models/mt/es-en");
        if !dir.join("encoder.onnx").exists() {
            eprintln!("omitido: sin modelos MT");
            return;
        }
        let mut t = MarianTranslator::new(dir, 4).unwrap();
        let (text, ms) = t.translate("gracias por llamar").unwrap();
        eprintln!("ES->EN '{text}' ({ms}ms)");
        assert!(text.to_lowercase().contains("thank"), "obtenido: {text}");
    }
}
