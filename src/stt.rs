#[cfg(test)]
use crate::vad::SpeechSegment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttLang {
    En,
    Es,
}

impl SttLang {
    pub fn code(self) -> &'static str {
        match self {
            SttLang::En => "en",
            SttLang::Es => "es",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SttResult {
    pub text: String,
}

pub trait Transcriber {
    fn transcribe(&mut self, samples_16k: &[f32], lang: SttLang) -> anyhow::Result<SttResult>;
}

/// Mock para cablear el pipeline sin modelo ni cmake.
/// Devuelve texto marcado como mock con la duración real del segmento.
#[cfg(test)]
pub struct MockTranscriber;

#[cfg(test)]
impl Transcriber for MockTranscriber {
    fn transcribe(&mut self, samples_16k: &[f32], lang: SttLang) -> anyhow::Result<SttResult> {
        let secs = samples_16k.len() as f32 / crate::vad::TARGET_SR as f32;
        // Simula ~50ms de cómputo para medir el flujo UI.
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(SttResult {
            text: format!("[mock {:.2}s {}]", secs, lang.code()),
        })
    }
}

#[cfg(test)]
pub fn transcribe_segment_mock(seg: &SpeechSegment, lang: SttLang) -> SttResult {
    MockTranscriber
        .transcribe(&seg.samples_16k, lang)
        .expect("mock nunca falla")
}

/// Rutas donde buscaremos modelos GGML locales (no se suben al repo).
#[allow(dead_code)]
pub fn model_candidates() -> Vec<std::path::PathBuf> {
    vec![
        std::path::PathBuf::from("models/ggml-base.bin"),
        std::path::PathBuf::from("models/ggml-small.bin"),
    ]
}

pub fn whisper_exe_candidate() -> std::path::PathBuf {
    std::path::PathBuf::from(
        "third_party/whisper.cpp/build/bin/Release/whisper-cli.exe",
    )
}

pub fn whisper_server_candidate() -> std::path::PathBuf {
    std::path::PathBuf::from(
        "third_party/whisper.cpp/build/bin/Release/whisper-server.exe",
    )
}

/// Modelo base por defecto (el mismo que usa el CLI).
pub fn base_model_path() -> Option<std::path::PathBuf> {
    model_candidates().into_iter().find(|p| p.exists())
}

/// Modelos tiny para streaming (rápidos, menos precisos que base).
pub fn tiny_model_path(lang: SttLang) -> Option<std::path::PathBuf> {
    let name = match lang {
        SttLang::En => "models/ggml-tiny.en.bin",
        SttLang::Es => "models/ggml-tiny.bin",
    };
    let p = std::path::PathBuf::from(name);
    p.exists().then_some(p)
}

/// Nombre temporal único por llamada (los dos workers comparten proceso).
fn temp_wav() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("freetouch_stt_{}_{n}.wav", std::process::id()))
}

/// STT real vía whisper-cli compilado localmente (CPU).
/// Escribe el segmento a WAV temporal y parsea el texto de stdout.
pub struct CliTranscriber {
    pub exe: std::path::PathBuf,
    pub model: std::path::PathBuf,
    pub threads: u32,
}

impl CliTranscriber {
    pub fn with_defaults() -> anyhow::Result<Self> {
        let exe = whisper_exe_candidate();
        if !exe.exists() {
            anyhow::bail!("falta {}", exe.display());
        }
        let model = base_model_path()
            .ok_or_else(|| anyhow::anyhow!("falta models/ggml-base.bin"))?;
        Ok(Self { exe, model, threads: 8 })
    }

    pub fn use_small_model(&mut self) -> bool {
        let p = std::path::PathBuf::from("models/ggml-small.bin");
        if p.exists() {
            self.model = p;
            true
        } else {
            false
        }
    }
}

impl Transcriber for CliTranscriber {
    fn transcribe(&mut self, samples_16k: &[f32], lang: SttLang) -> anyhow::Result<SttResult> {
        use std::process::Command;
        let wav = temp_wav();
        crate::wav::write_mono16(&wav, samples_16k, 16_000)?;
        let out = Command::new(&self.exe)
            .args([
                "-m",
                &self.model.to_string_lossy().to_string(),
                "-l",
                lang.code(),
                "-f",
                &wav.to_string_lossy().to_string(),
                "--no-timestamps",
                "--no-prints",
                "-t",
                &self.threads.to_string(),
            ])
            .output()?;
        let _ = std::fs::remove_file(&wav);
        if !out.status.success() {
            anyhow::bail!("whisper-cli falló: {}", String::from_utf8_lossy(&out.stderr));
        }
        let text_out = String::from_utf8_lossy(&out.stdout);
        // whisper-cli imprime logs a stderr y el texto al final de stdout.
        let text = text_out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .last()
            .unwrap_or("")
            .to_owned();
        Ok(SttResult { text })
    }
}

/// STT contra whisper-server local (modelo residente: sin recarga por frase).
/// El worker lanza un servidor por dirección (idioma y puerto propios) y
/// lo mata al terminar. Si el servidor no arranca, el worker usa el CLI.
pub struct ServerStt {
    child: Option<std::process::Child>,
    pub url: String,
}

impl ServerStt {
    /// Puerto por idioma para no chocar entre direcciones.
    pub fn port_for(lang: SttLang) -> u16 {
        match lang {
            SttLang::En => 18080,
            SttLang::Es => 18081,
        }
    }

    pub fn launch(lang: SttLang, threads: u32) -> anyhow::Result<Self> {
        let model = base_model_path().ok_or_else(|| anyhow::anyhow!("falta modelo base"))?;
        Self::launch_with_model(model, lang, threads, Self::port_for(lang))
    }

    pub fn launch_with_model(
        model: std::path::PathBuf,
        lang: SttLang,
        threads: u32,
        port: u16,
    ) -> anyhow::Result<Self> {
        let exe = whisper_server_candidate();
        if !exe.exists() {
            anyhow::bail!("falta {}", exe.display());
        }
        let mut child = std::process::Command::new(&exe)
            .args([
                "-m",
                &model.to_string_lossy().to_string(),
                "-l",
                lang.code(),
                "-t",
                &threads.to_string(),
                "--port",
                &port.to_string(),
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        // Espera a que acepte conexiones (carga del modelo).
        let addr = format!("127.0.0.1:{port}");
        let mut ready = false;
        for _ in 0..120 {
            if std::net::TcpStream::connect_timeout(
                &addr.parse().map_err(|_| anyhow::anyhow!("addr"))?,
                std::time::Duration::from_millis(500),
            )
            .is_ok()
            {
                ready = true;
                break;
            }
            if let Ok(Some(_)) = child.try_wait() {
                break; // murió al arrancar
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if !ready {
            let _ = child.kill();
            anyhow::bail!("whisper-server no respondió en el puerto {port}");
        }
        Ok(Self { child: Some(child), url: format!("http://{addr}/inference") })
    }

    /// Transcribe con segmentos y tiempos (verbose_json) para streaming.
    /// Devuelve (inicio_s, fin_s, texto) por segmento.
    pub fn post_wav_segments(
        &self,
        wav_bytes: &[u8],
    ) -> anyhow::Result<Vec<(f32, f32, String)>> {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let bound = "----freetouch7d3b9a1f";
        let mut body = format!(
            "--{bound}\r\nContent-Disposition: form-data; name=\"response_format\"\r\n\r\nverbose_json\r\n"
        );
        body.push_str(&format!(
            "--{bound}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"seg.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        ));
        let tail = format!("\r\n--{bound}--\r\n");
        let total = body.len() + wav_bytes.len() + tail.len();
        let host_port = self
            .url
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("127.0.0.1:18080");
        let req = format!(
            "POST /inference HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: multipart/form-data; boundary={bound}\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
        );
        let mut s = TcpStream::connect_timeout(
            &host_port
                .parse()
                .map_err(|_| anyhow::anyhow!("addr servidor"))?,
            std::time::Duration::from_secs(5),
        )?;
        s.set_write_timeout(Some(std::time::Duration::from_secs(30)))?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(180)))?;
        s.write_all(req.as_bytes())?;
        s.write_all(body.as_bytes())?;
        s.write_all(wav_bytes)?;
        s.write_all(tail.as_bytes())?;
        let mut resp = Vec::new();
        s.read_to_end(&mut resp)?;
        let text = String::from_utf8_lossy(&resp);
        let js = text.split("\r\n\r\n").last().unwrap_or("");
        let v: serde_json::Value = serde_json::from_str(js)?;
        let mut out = Vec::new();
        if let Some(segs) = v.get("segments").and_then(|x| x.as_array()) {
            for sg in segs {
                let a = sg.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let b = sg.get("end").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let t = sg.get("text").and_then(|x| x.as_str()).unwrap_or("").trim().to_owned();
                if !t.is_empty() && b > a {
                    out.push((a, b, t));
                }
            }
        }
        Ok(out)
    }

    fn post_wav(&self, wav_bytes: &[u8]) -> anyhow::Result<String> {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let bound = "----freetouch7d3b9a1f";
        let head = format!(
            "--{bound}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"seg.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        );
        let tail = format!("\r\n--{bound}--\r\n");
        let total = head.len() + wav_bytes.len() + tail.len();
        let host_port = self
            .url
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("127.0.0.1:18080");
        let req = format!(
            "POST /inference HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: multipart/form-data; boundary={bound}\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
        );
        let mut s = TcpStream::connect_timeout(
            &host_port
                .parse()
                .map_err(|_| anyhow::anyhow!("addr servidor"))?,
            std::time::Duration::from_secs(5),
        )?;
        s.set_write_timeout(Some(std::time::Duration::from_secs(30)))?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(180)))?;
        s.write_all(req.as_bytes())?;
        s.write_all(head.as_bytes())?;
        s.write_all(wav_bytes)?;
        s.write_all(tail.as_bytes())?;
        let mut resp = Vec::new();
        s.read_to_end(&mut resp)?;
        let text = String::from_utf8_lossy(&resp);
        let body = text.split("\r\n\r\n").last().unwrap_or("");
        let v: serde_json::Value = serde_json::from_str(body)?;
        Ok(v
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_owned())
    }
}

impl Drop for ServerStt {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Transcriber for ServerStt {
    fn transcribe(&mut self, samples_16k: &[f32], _lang: SttLang) -> anyhow::Result<SttResult> {
        let wav = temp_wav();
        crate::wav::write_mono16(&wav, samples_16k, 16_000)?;
        let bytes = std::fs::read(&wav)?;
        let _ = std::fs::remove_file(&wav);
        let text = self.post_wav(&bytes)?;
        Ok(SttResult { text })
    }
}

/// STT en streaming: ventana deslizante contra un servidor tiny.
/// Emite por hold-n: de cada transcripción completa se entregan todas
/// las palabras menos las últimas `hold` (inestables).
pub struct SttStream {
    server: ServerStt,
    sample_rate: u32,
    window_secs: f32,
    step_secs: f32,
    buf: Vec<f32>,
    win_start: f32,
    pending_secs: f32,
    stream_secs: f32,
    last_end: f32,
    emitted_tail: Vec<String>,
    pub last_ms: u128,
}

/// Solape: mayor k con cola emitida == inicio actual (insensible a caso).
fn overlap_skip(emitted: &[String], cur: &[String]) -> usize {
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

impl SttStream {
    /// Puertos propios para no chocar con los servidores base del worker.
    pub fn port_for_stream(lang: SttLang) -> u16 {
        match lang {
            SttLang::En => 19080,
            SttLang::Es => 19081,
        }
    }

    pub fn launch(
        lang: SttLang,
        window_secs: f32,
        step_secs: f32,
    ) -> anyhow::Result<Self> {
        let model =
            tiny_model_path(lang).ok_or_else(|| anyhow::anyhow!("falta modelo tiny"))?;
        let server =
            ServerStt::launch_with_model(model, lang, 8, Self::port_for_stream(lang))?;
        Ok(Self {
            server,
            sample_rate: 16_000,
            window_secs,
            step_secs,
            buf: Vec::new(),
            win_start: 0.0,
            pending_secs: 0.0,
            stream_secs: 0.0,
            last_end: 0.0,
            emitted_tail: Vec::new(),
            last_ms: 0,
        })
    }

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_owned()).collect()
    }

    fn take_tail(v: &mut Vec<String>, keep: usize) {
        if v.len() > keep {
            let drop = v.len() - keep;
            v.drain(0..drop);
        }
    }

    /// Devuelve las palabras recién estabilizadas (vacío la mayoría de
    /// las veces). `samples_16k` es mono a 16kHz. Emite los segmentos
    /// cerrados (no el último, aún abierto) que aporten audio nuevo.
    pub fn push(&mut self, samples_16k: &[f32]) -> Vec<String> {
        self.buf.extend_from_slice(samples_16k);
        self.stream_secs += samples_16k.len() as f32 / self.sample_rate as f32;
        self.pending_secs += samples_16k.len() as f32 / self.sample_rate as f32;
        // Recorta a la ventana (el tiempo absoluto del inicio avanza).
        let max_len = (self.window_secs * self.sample_rate as f32) as usize;
        if self.buf.len() > max_len {
            let drop = self.buf.len() - max_len;
            self.buf.drain(0..drop);
            self.win_start += drop as f32 / self.sample_rate as f32;
        }
        if self.pending_secs < self.step_secs {
            return Vec::new();
        }
        self.pending_secs = 0.0;
        let t0 = std::time::Instant::now();
        let rms = (self.buf.iter().map(|s| s * s).sum::<f32>()
            / self.buf.len().max(1) as f32)
            .sqrt();
        let segs = match self.infer_segments() {
            Ok(s) => s,
            Err(e) => {
                if rms > 0.02 {
                    eprintln!("STT stream err con audio (rms={rms:.3}): {e:#}");
                }
                return Vec::new();
            }
        };
        self.last_ms = t0.elapsed().as_millis();
        let win_end = self.win_start + self.buf.len() as f32 / self.sample_rate as f32;
        let mut fresh: Vec<String> = Vec::new();
        let n = segs.len();
        for (i, (_a, b, t)) in segs.iter().enumerate() {
            let abs_b = self.win_start + b;
            let is_last = i + 1 == n;
            // El último segmento está abierto, salvo que haya >=1s de
            // audio detrás (entonces ya se cerró).
            if is_last && win_end - abs_b < 1.0 {
                continue;
            }
            if abs_b <= self.last_end {
                continue;
            }
            let words = Self::words(t);
            if words.is_empty() {
                continue;
            }
            let skip = overlap_skip(&self.emitted_tail, &words);
            fresh.extend(words[skip..].iter().cloned());
            self.last_end = self.last_end.max(abs_b);
        }
        for w in &fresh {
            self.emitted_tail.push(w.clone());
        }
        Self::take_tail(&mut self.emitted_tail, 20);
        if fresh.is_empty() && rms > 0.02 {
            eprintln!("STT stream: audio (rms={rms:.3}) sin segmentos nuevos");
        }
        fresh
    }

    fn infer_segments(&mut self) -> anyhow::Result<Vec<(f32, f32, String)>> {
        let wav = temp_wav();
        crate::wav::write_mono16(&wav, &self.buf, 16_000)?;
        let bytes = std::fs::read(&wav)?;
        let _ = std::fs::remove_file(&wav);
        self.server.post_wav_segments(&bytes)
    }

    /// Al cerrar: emite todo lo pendiente incluido el último segmento.
    pub fn flush(&mut self) -> Vec<String> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        let t0 = std::time::Instant::now();
        let segs = match self.infer_segments() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        self.last_ms = t0.elapsed().as_millis();
        let mut fresh: Vec<String> = Vec::new();
        for (_a, b, t) in &segs {
            let abs_b = self.win_start + b;
            if abs_b <= self.last_end {
                continue;
            }
            let words = Self::words(t);
            let skip = overlap_skip(&self.emitted_tail, &words);
            fresh.extend(words[skip.min(words.len())..].iter().cloned());
            self.last_end = self.last_end.max(abs_b);
        }
        for w in &fresh {
            self.emitted_tail.push(w.clone());
        }
        Self::take_tail(&mut self.emitted_tail, 20);
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_duration_and_lang() {
        let seg = SpeechSegment {
            samples_16k: vec![0.0; 16_000],
        };
        let r = transcribe_segment_mock(&seg, SttLang::En);
        assert!(r.text.contains("1.00s"));
        assert!(r.text.contains("en"));
    }

    /// Streaming largo: alimenta test_es.wav completo en pasos de 0.5s
    /// y verifica que salen palabras (detecta fallos en vivo).
    #[test]
    fn stream_long_audio_emits() {
        let wav = std::path::PathBuf::from("voices/test_es.wav");
        if !wav.exists() {
            eprintln!("omitido: sin test_es.wav");
            return;
        }
        let w = crate::wav::read_mono_as_f32(&wav).unwrap();
        let samples = crate::wav::resample_to_16k(&w.samples, w.sample_rate);
        eprintln!("muestras: {}", samples.len());
        let mut st = SttStream::launch(SttLang::Es, 4.0, 0.5).unwrap();
        let mut got: Vec<String> = Vec::new();
        let step = 8_000;
        let mut i = 0;
        let mut n_inf = 0;
        while i < samples.len() {
            let end = (i + step).min(samples.len());
            let before = got.len();
            for w in st.push(&samples[i..end]) {
                got.push(w);
            }
            if got.len() > before {
                n_inf += 1;
            }
            i = end;
        }
        for w in st.flush() {
            got.push(w);
        }
        eprintln!("rondas con emision: {n_inf}, total palabras: {}", got.len());
        eprintln!("texto: {}", got.join(" "));
        assert!(!got.is_empty(), "cero palabras en audio real");
    }

    /// Streaming real contra servidor tiny: alimenta test_en.wav en
    /// pasos de 0.5s y mide cuándo salen las palabras estables.
    #[test]
    fn stream_emits_hello_over_time() {
        let wav = std::path::PathBuf::from("voices/test_en.wav");
        if !wav.exists() {
            eprintln!("omitido: sin test_en.wav");
            return;
        }
        let w = crate::wav::read_mono_as_f32(&wav).unwrap();
        let samples = crate::wav::resample_to_16k(&w.samples, w.sample_rate);
        let mut st = SttStream::launch(SttLang::En, 4.0, 0.5).unwrap();
        let t0 = std::time::Instant::now();
        let mut got: Vec<String> = Vec::new();
        let step = 8_000; // 0.5s a 16k
        let mut i = 0;
        while i < samples.len() {
            let end = (i + step).min(samples.len());
            for w in st.push(&samples[i..end]) {
                if !got.contains(&w) {
                    eprintln!("[{:.1}s] {}", t0.elapsed().as_secs_f32(), w);
                }
                got.push(w);
            }
            i = end;
        }
        for w in st.flush() {
            eprintln!("[{:.1}s] (flush) {}", t0.elapsed().as_secs_f32(), w);
            got.push(w);
        }
        let joined = got.join(" ").to_lowercase();
        eprintln!("total: {joined}");
        assert!(
            joined.contains("hello"),
            "no salió 'hello', obtenido: {joined}"
        );
    }
}
