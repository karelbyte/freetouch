//! Medición puntual del nivel que llega al Cable A (temporal).
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "medicion manual: requiere audio sonando en el Cable A durante 10s"]
fn measure_cable_a_input_level() {
    let host = cpal::default_host();
    let dev = host
        .input_devices()
        .unwrap()
        .find(|d| {
            d.description()
                .map(|x| x.name().contains("Micrófono (FreeTouch Cable A)"))
                .unwrap_or(false)
        })
        .expect("sin micro cable A");
    let cfg: cpal::StreamConfig = dev.default_input_config().unwrap().into();
    let rate = cfg.sample_rate;
    let nch = cfg.channels as usize;
    eprintln!("formato captura: {rate}Hz {nch}ch");
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let cb = buf.clone();
    let stream = dev
        .build_input_stream(
            cfg,
            move |data: &[f32], _| {
                cb.lock().unwrap().extend_from_slice(data);
            },
            |e| eprintln!("err: {e}"),
            None,
        )
        .unwrap();
    stream.play().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(10));
    drop(stream);
    let data = buf.lock().unwrap().clone();
    eprintln!("muestras: {} ({}s)", data.len(), data.len() as f32 / rate as f32);
    // Guarda WAV PCM16 para escuchar la fidelidad real.
    {
        let mut frames = data.clone();
        while frames.len() % nch != 0 {
            frames.push(0.0);
        }
        let nframes = (frames.len() / nch) as u32;
        let pcm: Vec<i16> = frames
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        let data_bytes = (pcm.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + data_bytes as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&(nch as u16).to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * nch as u32 * 2).to_le_bytes());
        wav.extend_from_slice(&((nch * 2) as u16).to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        for s in &pcm {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        let path = std::env::temp_dir().join("cableA-sample.wav");
        std::fs::write(&path, &wav).unwrap();
        eprintln!("wav guardado: {}", path.display());
    }
    let rms_all =
        (data.iter().map(|s| s * s).sum::<f32>() / data.len().max(1) as f32).sqrt();
    // RMS máximo por ventanas de 100ms.
    let win = (rate as usize / 10).max(1);
    let mut max_rms = 0.0f32;
    for w in data.chunks(win) {
        let r = (w.iter().map(|s| s * s).sum::<f32>() / w.len().max(1) as f32).sqrt();
        if r > max_rms {
            max_rms = r;
        }
    }
    let peak = data.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    eprintln!("RMS global={rms_all:.4} RMS max100ms={max_rms:.4} pico={peak:.3}");
    eprintln!("(umbral VAD actual: 0.035)");
}
