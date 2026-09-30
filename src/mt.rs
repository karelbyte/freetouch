#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MtDir {
    EnToEs,
    EsToEn,
}

impl MtDir {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            MtDir::EnToEs => "EN->ES",
            MtDir::EsToEn => "ES->EN",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MtResult {
    pub text: String,
}

pub trait Translator {
    fn translate(&mut self, text: &str, dir: MtDir) -> anyhow::Result<MtResult>;
    /// Retraduce con prefijo forzado (streaming). Por defecto ignora el
    /// prefijo; el traductor real lo impone para salida monótona.
    fn translate_prefix(
        &mut self,
        text: &str,
        dir: MtDir,
        _prefix_words: &[String],
    ) -> anyhow::Result<MtResult> {
        self.translate(text, dir)
    }
}

/// Traductor real MarianMT en ONNX (local, CPU).
pub struct RealTranslator {
    en_es: crate::mt_onnx::MarianTranslator,
    es_en: crate::mt_onnx::MarianTranslator,
}

impl RealTranslator {
    pub fn with_defaults(threads: usize) -> anyhow::Result<Self> {
        Ok(Self {
            en_es: crate::mt_onnx::MarianTranslator::new(
                std::path::Path::new("models/mt/en-es"),
                threads,
            )?,
            es_en: crate::mt_onnx::MarianTranslator::new(
                std::path::Path::new("models/mt/es-en"),
                threads,
            )?,
        })
    }
}

impl Translator for RealTranslator {
    fn translate(&mut self, text: &str, dir: MtDir) -> anyhow::Result<MtResult> {
        let (t, _) = match dir {
            MtDir::EnToEs => self.en_es.translate(text)?,
            MtDir::EsToEn => self.es_en.translate(text)?,
        };
        Ok(MtResult { text: t })
    }

    fn translate_prefix(
        &mut self,
        text: &str,
        dir: MtDir,
        prefix_words: &[String],
    ) -> anyhow::Result<MtResult> {
        let mt = match dir {
            MtDir::EnToEs => &mut self.en_es,
            MtDir::EsToEn => &mut self.es_en,
        };
        let (t, _) = mt.translate_with_prefix(text, prefix_words)?;
        Ok(MtResult { text: t })
    }
}

/// Traductor mock con mini-diccionario para cablear el pipeline
/// VAD -> STT -> MT sin modelos. Marca salida como [mock].
/// El traductor real (Marian/ONNX) irá tras feature `mt-onnx`.
pub struct MockTranslator;

fn dict_lookup(lower: &str, dir: MtDir) -> Option<&'static str> {
    match dir {
        MtDir::EnToEs => match lower {
            "hello" => Some("hola"),
            "how are you" => Some("cómo estás"),
            "thank you" => Some("gracias"),
            "good morning" => Some("buenos días"),
            "goodbye" => Some("adiós"),
            _ => None,
        },
        MtDir::EsToEn => match lower {
            "hola" => Some("hello"),
            "cómo estás" | "como estas" => Some("how are you"),
            "gracias" => Some("thank you"),
            "buenos días" | "buenos dias" => Some("good morning"),
            "adiós" | "adios" => Some("goodbye"),
            _ => None,
        },
    }
}

impl Translator for MockTranslator {
    fn translate(&mut self, text: &str, dir: MtDir) -> anyhow::Result<MtResult> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            anyhow::bail!("texto vacío");
        }
        let lower = trimmed.to_lowercase();
        let out = dict_lookup(&lower, dir)
            .map(|s| s.to_owned())
            .unwrap_or_else(|| match dir {
                MtDir::EnToEs => format!("[ES mock] {trimmed}"),
                MtDir::EsToEn => format!("[EN mock] {trimmed}"),
            });
        Ok(MtResult { text: out })
    }
}

/// MT en streaming por retraducción: ante palabras nuevas se retraduce
/// el prefijo completo y se emite lo estable (alineado con lo ya
/// entregado), reteniendo las últimas `hold` palabras (inestables).
pub struct MtStream {
    translator: Box<dyn Translator + Send>,
    dir: MtDir,
    src_words: Vec<String>,
    emitted: Vec<String>,
    hold: usize,
    since_run: usize,
    pub last_ms: u128,
}

impl MtStream {
    pub fn new(translator: Box<dyn Translator + Send>, dir: MtDir, hold: usize) -> Self {
        Self { translator, dir, src_words: Vec::new(), emitted: Vec::new(), hold, since_run: 0, last_ms: 0 }
    }

    /// Agrega palabras fuente; cada 3+ nuevas retraduce y devuelve las
    /// palabras destino recién estabilizadas.
    pub fn push(&mut self, words: &[String]) -> Vec<String> {
        self.src_words.extend(words.iter().cloned());
        self.since_run += words.len();
        if self.since_run < 3 {
            return Vec::new();
        }
        self.since_run = 0;
        self.retranslate()
    }

    /// Quita de `cur` lo ya emitido: mayor solape entre la cola emitida
    /// y el inicio de `cur` (tolerante a la costura por re-segmentación
    /// BPE del prefijo forzado). Sin solape claro devuelve 0.
    fn strip_known(emitted: &[String], cur: &[String]) -> usize {
        let maxk = emitted.len().min(cur.len());
        for k in (1..=maxk).rev() {
            let tail = &emitted[emitted.len() - k..];
            if cur[..k]
                .iter()
                .zip(tail.iter())
                .all(|(a, b)| a.to_lowercase() == b.to_lowercase())
            {
                return k;
            }
        }
        0
    }

    fn retranslate(&mut self) -> Vec<String> {
        let src = self.src_words.join(" ");
        let t0 = std::time::Instant::now();
        let out = match self.translator.translate_prefix(&src, self.dir, &self.emitted) {
            Ok(r) => r.text,
            Err(_) => return Vec::new(),
        };
        self.last_ms = t0.elapsed().as_millis();
        let cur: Vec<String> = out.split_whitespace().map(|w| w.to_owned()).collect();
        let start = Self::strip_known(&self.emitted, &cur);
        let end = cur.len().saturating_sub(self.hold);
        if end <= start {
            return Vec::new();
        }
        let fresh: Vec<String> = cur[start..end].to_vec();
        self.emitted.extend(fresh.iter().cloned());
        if self.emitted.len() > 60 {
            let drop = self.emitted.len() - 60;
            self.emitted.drain(0..drop);
        }
        fresh
    }

    /// Al cerrar frase: emite todo lo pendiente incluido lo retenido.
    pub fn flush(&mut self) -> Vec<String> {
        let src = self.src_words.join(" ");
        self.src_words.clear();
        self.since_run = 0;
        if src.trim().is_empty() {
            return Vec::new();
        }
        let t0 = std::time::Instant::now();
        let out = match self.translator.translate_prefix(&src, self.dir, &self.emitted) {
            Ok(r) => r.text,
            Err(_) => return Vec::new(),
        };
        self.last_ms = t0.elapsed().as_millis();
        let cur: Vec<String> = out.split_whitespace().map(|w| w.to_owned()).collect();
        let start = Self::strip_known(&self.emitted, &cur);
        let fresh: Vec<String> = cur[start..].to_vec();
        self.emitted.extend(fresh.iter().cloned());
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_to_es_dictionary() {
        let mut t = MockTranslator;
        let r = t.translate("hello", MtDir::EnToEs).unwrap();
        assert_eq!(r.text, "hola");
    }

    #[test]
    fn es_to_en_dictionary() {
        let mut t = MockTranslator;
        let r = t.translate("gracias", MtDir::EsToEn).unwrap();
        assert_eq!(r.text, "thank you");
    }

    #[test]
    fn unknown_falls_back_with_prefix() {
        let mut t = MockTranslator;
        let r = t.translate("see you later", MtDir::EnToEs).unwrap();
        assert!(r.text.starts_with("[ES mock]"));
    }

    #[test]
    fn empty_is_error() {
        let mut t = MockTranslator;
        assert!(t.translate("   ", MtDir::EnToEs).is_err());
    }

    /// Streaming real: alimenta por trozos y verifica que salen
    /// palabras estables en español + flush final.
    #[test]
    fn stream_retranslate_stabilizes() {
        let rt = RealTranslator::with_defaults(4).unwrap();
        let mut st = MtStream::new(Box::new(rt), MtDir::EnToEs, 3);
        let mut got: Vec<String> = Vec::new();
        for chunk in [
            "Hello and welcome".split_whitespace(),
            "to the Kubernetes crash".split_whitespace(),
            "course today".split_whitespace(),
        ] {
            let w: Vec<String> = chunk.map(|s| s.to_owned()).collect();
            for x in st.push(&w) {
                eprintln!("MT> {x}");
                got.push(x);
            }
        }
        for x in st.flush() {
            eprintln!("MT-final> {x}");
            got.push(x);
        }
        let joined = got.join(" ").to_lowercase();
        eprintln!("MT total: {joined}");
        assert!(
            joined.contains("hola") || joined.contains("bienvenido"),
            "sin español estable: {joined}"
        );
    }
}
