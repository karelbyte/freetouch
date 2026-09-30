use anyhow::{Context, bail};

/// WAV PCM16 mono mínimo para intercambiar audio con CLIs locales
/// (whisper-cli lee WAV, piper escribe WAV).

pub fn write_mono16(path: &std::path::Path, samples: &[f32], sample_rate: u32) -> anyhow::Result<()> {
    let mut pcm: Vec<u8> = Vec::with_capacity(samples.len() * 2 + 44);
    let data_len = (samples.len() * 2) as u32;
    // Header RIFF
    pcm.extend_from_slice(b"RIFF");
    pcm.extend_from_slice(&(36 + data_len).to_le_bytes());
    pcm.extend_from_slice(b"WAVEfmt ");
    pcm.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    pcm.extend_from_slice(&1u16.to_le_bytes()); // PCM
    pcm.extend_from_slice(&1u16.to_le_bytes()); // mono
    pcm.extend_from_slice(&sample_rate.to_le_bytes());
    pcm.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    pcm.extend_from_slice(&2u16.to_le_bytes()); // block align
    pcm.extend_from_slice(&16u16.to_le_bytes()); // bits
    pcm.extend_from_slice(b"data");
    pcm.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, pcm)?;
    Ok(())
}

pub struct WavMono {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

pub fn read_mono_as_f32(path: &std::path::Path) -> anyhow::Result<WavMono> {
    let bytes = std::fs::read(path).with_context(|| format!("leyendo {}", path.display()))?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("no es WAV RIFF: {}", path.display());
    }
    // Busca chunks fmt y data (piper escribe fmt + data estándar).
    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data: &[u8] = &[];
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if id == b"fmt " && size >= 16 {
            channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap());
            sample_rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
            bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
        } else if id == b"data" {
            data = &bytes[body..(body + size).min(bytes.len())];
            break;
        }
        pos = body + size + (size % 2);
    }
    if sample_rate == 0 || data.is_empty() {
        bail!("WAV sin fmt/data: {}", path.display());
    }
    if bits != 16 {
        bail!("solo PCM16, hallado {bits} bits");
    }
    let ch = channels.max(1) as usize;
    let n_frames = data.len() / 2 / ch;
    let mut samples = Vec::with_capacity(n_frames);
    for i in 0..n_frames {
        let mut acc = 0.0f32;
        for c in 0..ch {
            let off = (i * ch + c) * 2;
            let v = i16::from_le_bytes([data[off], data[off + 1]]);
            acc += v as f32 / i16::MAX as f32;
        }
        samples.push(acc / ch as f32);
    }
    Ok(WavMono { sample_rate, samples })
}

/// Remuestreo lineal mono a 16kHz.
pub fn resample_to_16k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == 16_000 || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = 16_000.0 / src_rate as f64;
    let out_len = ((samples.len() as f64 * ratio) as usize).max(1);
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 / ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = samples.get(idx).copied().unwrap_or(0.0);
        let b = samples.get(idx + 1).copied().unwrap_or(0.0);
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_wav() {
        let dir = std::env::temp_dir();
        let p = dir.join("freetouch_wav_test.wav");
        let src: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0) * 0.5).collect();
        write_mono16(&p, &src, 16_000).unwrap();
        let w = read_mono_as_f32(&p).unwrap();
        assert_eq!(w.sample_rate, 16_000);
        assert_eq!(w.samples.len(), src.len());
        assert!((w.samples[800] - src[800]).abs() < 0.001);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn resample_22050_to_16k() {
        let src = vec![0.5f32; 22050];
        let out = resample_to_16k(&src, 22050);
        assert!((out.len() as i32 - 16_000).abs() < 50, "got {}", out.len());
    }
}
