#[cfg(test)]
pub const TTS_SR: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsLang {
    En,
    Es,
}

#[derive(Debug, Clone)]
pub struct TtsAudio {
    /// Mono a 16kHz f32, listo para `audio::BufferPlayer`.
    pub samples_16k: Vec<f32>,
}

pub trait Synthesizer {
    fn synthesize(&mut self, text: &str, lang: TtsLang) -> anyhow::Result<TtsAudio>;
}

/// Mock que genera blips por carácter para probar el flujo
/// MT -> audio sin voces ni ONNX. Frecuencia distinta por idioma.
#[cfg(test)]
pub struct MockSynthesizer;

#[cfg(test)]
impl Synthesizer for MockSynthesizer {
    fn synthesize(&mut self, text: &str, lang: TtsLang) -> anyhow::Result<TtsAudio> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            anyhow::bail!("texto vacío");
        }
        let freq = match lang {
            TtsLang::En => 220.0f32,
            TtsLang::Es => 260.0f32,
        };
        let per_char = (TTS_SR as usize * 40) / 1000; // 40ms por carácter
        let fade = (TTS_SR as usize * 5) / 1000; // 5ms fade
        let mut out = Vec::with_capacity(trimmed.len() * per_char);
        for (ci, _ch) in trimmed.chars().enumerate() {
            for i in 0..per_char {
                let t = (ci * per_char + i) as f32 / TTS_SR as f32;
                let mut s =
                    (t * freq * 2.0 * std::f32::consts::PI).sin() * 0.25;
                let pos = i;
                if pos < fade {
                    s *= pos as f32 / fade as f32;
                } else if pos >= per_char - fade {
                    s *= (per_char - pos) as f32 / fade as f32;
                }
                out.push(s);
            }
        }
        Ok(TtsAudio { samples_16k: out })
    }
}

/// Voces masculinas locales (Piper). No se suben al repo.
pub fn voice_for(lang: TtsLang) -> (std::path::PathBuf, Option<u32>) {
    match lang {
        // Lessac (EN, masculina, un solo hablante).
        TtsLang::En => (
            std::path::PathBuf::from("voices/en_US-lessac-medium.onnx"),
            None,
        ),
        // Sharvard (ES, hablante 0 = M).
        TtsLang::Es => (
            std::path::PathBuf::from("voices/es_ES-sharvard-medium.onnx"),
            Some(0),
        ),
    }
}

/// TTS real vía Piper local (CLI Python). Voz masculina en ambos idiomas.
pub struct PiperSynthesizer {
    pub piper_cmd: String,
}

impl PiperSynthesizer {
    pub fn with_defaults() -> anyhow::Result<Self> {
        let (en_voice, _) = voice_for(TtsLang::En);
        let (es_voice, _) = voice_for(TtsLang::Es);
        if !en_voice.exists() {
            anyhow::bail!("falta {}", en_voice.display());
        }
        if !es_voice.exists() {
            anyhow::bail!("falta {}", es_voice.display());
        }
        Ok(Self { piper_cmd: "piper".to_owned() })
    }
}

impl Synthesizer for PiperSynthesizer {
    fn synthesize(&mut self, text: &str, lang: TtsLang) -> anyhow::Result<TtsAudio> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let trimmed = text.trim();
        if trimmed.is_empty() {
            anyhow::bail!("texto vacío");
        }
        let (voice, speaker) = voice_for(lang);
        let wav = std::env::temp_dir().join(format!("freetouch_tts_{}.wav", std::process::id()));
        let mut args = vec![
            "-m".to_owned(),
            voice.to_string_lossy().to_string(),
            "-f".to_owned(),
            wav.to_string_lossy().to_string(),
        ];
        if let Some(s) = speaker {
            args.push("--speaker".to_owned());
            args.push(s.to_string());
        }
        // En Windows `piper` es un .bat: se invoca vía cmd /C.
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", &self.piper_cmd]);
        cmd.args(&args);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("sin stdin"))?
            .write_all(trimmed.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            anyhow::bail!("piper falló: {}", String::from_utf8_lossy(&out.stderr));
        }
        let w = crate::wav::read_mono_as_f32(&wav)?;
        let _ = std::fs::remove_file(&wav);
        let samples_16k = crate::wav::resample_to_16k(&w.samples, w.sample_rate);
        Ok(TtsAudio { samples_16k })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_error() {
        let mut s = MockSynthesizer;
        assert!(s.synthesize("   ", TtsLang::Es).is_err());
    }

    #[test]
    fn duration_scales_with_text() {
        let mut s = MockSynthesizer;
        let a = s.synthesize("hi", TtsLang::En).unwrap();
        let b = s.synthesize("hello world", TtsLang::En).unwrap();
        assert!(b.samples_16k.len() > a.samples_16k.len());
        let secs = a.samples_16k.len() as f32 / TTS_SR as f32;
        assert!(secs > 0.05 && secs < 0.3, "secs {secs}");
    }

    #[test]
    fn lang_changes_tone() {
        let mut s = MockSynthesizer;
        let a = s.synthesize("hola", TtsLang::En).unwrap();
        let b = s.synthesize("hola", TtsLang::Es).unwrap();
        assert_eq!(a.samples_16k.len(), b.samples_16k.len());
        assert_ne!(a.samples_16k[100].to_bits(), b.samples_16k[100].to_bits());
    }
}
