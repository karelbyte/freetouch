use webrtc_vad::{SampleRate, Vad, VadMode};

pub const TARGET_SR: u32 = 16_000;
/// 30ms a 16kHz, tamaño que acepta WebRTC VAD.
pub const FRAME_SAMPLES: usize = 480;

#[derive(Debug, Clone)]
pub struct SpeechSegment {
    /// Mono a 16kHz en f32 [-1, 1], listo para Whisper.
    pub samples_16k: Vec<f32>,
}

impl SpeechSegment {
    #[cfg(test)]
    pub fn duration_secs(&self) -> f32 {
        self.samples_16k.len() as f32 / TARGET_SR as f32
    }
}

/// Mezcla a mono y remuestrea a 16kHz con interpolación lineal.
/// Suficiente para VAD/STT en CPU sin dependencias pesadas.
pub struct Resampler16k {
    src_rate: u32,
    channels: usize,
    buf: Vec<f32>,
    phase: f64,
}

impl Resampler16k {
    pub fn new(src_rate: u32, channels: usize) -> Self {
        Self {
            src_rate: src_rate.max(1),
            channels: channels.max(1),
            buf: Vec::new(),
            phase: 0.0,
        }
    }

    /// `frames` es intercalado (LRLR...) en f32 a `src_rate`.
    /// Acumula salida 16k mono en `out`.
    pub fn push_interleaved(&mut self, frames: &[f32], out: &mut Vec<f32>) {
        if frames.is_empty() || self.channels == 0 {
            return;
        }
        let n_frames = frames.len() / self.channels;
        for i in 0..n_frames {
            let mut acc = 0.0f32;
            for ch in 0..self.channels {
                acc += frames[i * self.channels + ch];
            }
            self.buf.push(acc / self.channels as f32);
        }

        if self.src_rate == TARGET_SR {
            out.extend(self.buf.drain(..));
            return;
        }

        let step = self.src_rate as f64 / TARGET_SR as f64;
        while (self.phase + 1.0) < self.buf.len() as f64 {
            let idx = self.phase.floor() as usize;
            let frac = (self.phase - idx as f64) as f32;
            let a = self.buf[idx];
            let b = self.buf[idx + 1];
            out.push(a + (b - a) * frac);
            self.phase += step;
        }
        // Descarta lo consumido, conserva 2 muestras para interpolar.
        let consumed = (self.phase.floor() as usize).saturating_sub(1);
        if consumed > 0 {
            let keep_from = consumed.min(self.buf.len().saturating_sub(2));
            self.buf.drain(0..keep_from);
            self.phase -= keep_from as f64;
        }
    }
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

/// Segmentador por frases: acumula voz y corta con ~600ms de silencio.
/// Decisión = WebRTC VAD OR energía, para no perder inicios en CPU
/// ruidosa. Pensado para alimentar Whisper por frases cortas.
pub struct PhraseSegmenter {
    vad: Vad,
    resampler: Resampler16k,
    frame_buf: Vec<f32>,
    preroll: Vec<Vec<f32>>,
    in_speech: bool,
    current: Vec<f32>,
    silence_frames: usize,
    noise_frames: usize,
    silence_timeout_frames: usize,
    min_phrase_samples: usize,
    max_phrase_samples: usize,
    energy_threshold: f32,
}

impl PhraseSegmenter {
    pub fn new(src_rate: u32, channels: usize) -> Self {
        let vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive);
        Self {
            vad,
            resampler: Resampler16k::new(src_rate, channels),
            frame_buf: Vec::with_capacity(FRAME_SAMPLES * 2),
            preroll: Vec::new(),
            in_speech: false,
            current: Vec::new(),
            silence_frames: 0,
            noise_frames: 0,
            silence_timeout_frames: 24, // ~700ms con frames de 30ms
            min_phrase_samples: 8_000,  // 0.5s
            max_phrase_samples: TARGET_SR as usize * 8,
            energy_threshold: 0.035,
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.in_speech
    }

    /// (voz según WebRTC, voz solo por energía).
    fn voice_parts(&mut self, frame_16k: &[f32]) -> (bool, bool) {
        debug_assert_eq!(frame_16k.len(), FRAME_SAMPLES);
        let pcm16: Vec<i16> = frame_16k
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        let vad_voice = self.vad.is_voice_segment(&pcm16).unwrap_or(false);
        let energy_voice = rms(frame_16k) > self.energy_threshold;
        (vad_voice, energy_voice)
    }

    /// Devuelve los segmentos completos detectados en este push.
    pub fn push(&mut self, interleaved_src: &[f32]) -> Vec<SpeechSegment> {
        let mut out_16k = Vec::new();
        self.resampler.push_interleaved(interleaved_src, &mut out_16k);
        self.frame_buf.extend(out_16k);

        let mut emitted = Vec::new();
        while self.frame_buf.len() >= FRAME_SAMPLES {
            let frame: Vec<f32> = self.frame_buf.drain(0..FRAME_SAMPLES).collect();
            if let Some(seg) = self.push_frame(frame) {
                emitted.push(seg);
            }
        }
        emitted
    }

    fn push_frame(&mut self, frame: Vec<f32>) -> Option<SpeechSegment> {
        let (vad_voice, energy_voice) = self.voice_parts(&frame);
        let voice = vad_voice || energy_voice;
        if voice {
            // Ruido sostenido (efectos/música fuerte sin voz WebRTC):
            // se aborta el segmento en vez de alimentar al STT con ruido.
            if !vad_voice {
                self.noise_frames += 1;
                if self.noise_frames > 10 {
                    self.in_speech = false;
                    self.silence_frames = 0;
                    self.noise_frames = 0;
                    self.current.clear();
                    self.preroll.clear();
                    return None;
                }
            } else {
                self.noise_frames = 0;
            }
            if !self.in_speech {
                self.in_speech = true;
                self.silence_frames = 0;
                // Pre-roll para no recortar el ataque.
                for pre in self.preroll.drain(..) {
                    self.current.extend(pre);
                }
            } else {
                self.silence_frames = 0;
            }
            self.current.extend(frame);
            if self.current.len() >= self.max_phrase_samples {
                return self.close_phrase();
            }
            None
        } else if self.in_speech {
            self.current.extend(frame);
            self.silence_frames += 1;
            if self.silence_frames >= self.silence_timeout_frames {
                return self.close_phrase();
            }
            None
        } else {
            self.preroll.push(frame);
            if self.preroll.len() > 3 {
                self.preroll.remove(0);
            }
            None
        }
    }

    fn close_phrase(&mut self) -> Option<SpeechSegment> {
        self.in_speech = false;
        self.silence_frames = 0;
        self.noise_frames = 0;
        self.preroll.clear();
        if self.current.len() >= self.min_phrase_samples {
            let samples = std::mem::take(&mut self.current);
            Some(SpeechSegment { samples_16k: samples })
        } else {
            self.current.clear();
            None
        }
    }

    /// Cierra la frase en curso aunque no haya habido silencio final.
    #[cfg(test)]
    pub fn flush(&mut self) -> Option<SpeechSegment> {
        if self.current.len() >= self.min_phrase_samples {
            self.in_speech = false;
            self.silence_frames = 0;
            let samples = std::mem::take(&mut self.current);
            Some(SpeechSegment { samples_16k: samples })
        } else {
            self.current.clear();
            self.in_speech = false;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_produces_no_segments() {
        let mut seg = PhraseSegmenter::new(16_000, 1);
        let zeros = vec![0.0f32; 16_000 * 2];
        let out = seg.push(&zeros);
        assert!(out.is_empty());
        assert!(seg.flush().is_none());
    }

    #[test]
    fn resampler_48k_stereo_keeps_duration() {
        let mut r = Resampler16k::new(48_000, 2);
        let mut out = Vec::new();
        let one_sec = vec![0.1f32; 48_000 * 2];
        r.push_interleaved(&one_sec, &mut out);
        // 1s a 16k = 16000 muestras, tolerancia por fase.
        assert!((out.len() as i32 - 16_000).abs() < 200, "got {}", out.len());
    }

    #[test]
    fn loud_tone_produces_one_phrase() {
        let mut seg = PhraseSegmenter::new(48_000, 2);
        let mut emitted = Vec::new();
        for _ in 0..10 {
            emitted.extend(seg.push(&vec![0.0f32; 4800 * 2]));
        }
        for n in 0..10 {
            let mut buf = Vec::with_capacity(4800 * 2);
            for i in 0..4800 {
                let t = (n * 4800 + i) as f32 / 48_000.0;
                let s = (t * 200.0 * 2.0 * std::f32::consts::PI).sin() * 0.3;
                buf.push(s);
                buf.push(s);
            }
            emitted.extend(seg.push(&buf));
        }
        for _ in 0..25 {
            emitted.extend(seg.push(&vec![0.0f32; 4800 * 2]));
        }
        if let Some(s) = seg.flush() {
            emitted.push(s);
        }
        assert_eq!(emitted.len(), 1, "got {}", emitted.len());
        let d = emitted[0].duration_secs();
        assert!(d > 0.8 && d < 2.5, "duration {d}");
    }
}
