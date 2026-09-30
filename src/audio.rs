use anyhow::{Context, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

pub fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|desc| desc.name().to_owned())
}

pub fn list_devices() -> (Vec<String>, Vec<String>) {
    let host = cpal::default_host();
    let inputs = host
        .input_devices()
        .map(|devs| devs.filter_map(|d| device_name(&d)).collect::<Vec<_>>())
        .unwrap_or_default();
    let outputs = host
        .output_devices()
        .map(|devs| devs.filter_map(|d| device_name(&d)).collect::<Vec<_>>())
        .unwrap_or_default();
    (inputs, outputs)
}

#[cfg(test)]
mod probe {
    use super::*;

    #[test]
    fn probe_session_unmute() {
        use cpal::traits::{DeviceTrait, HostTrait};
        let (_, outputs) = list_devices();
        let dev = outputs
            .iter()
            .find(|d| d.contains("Altavoces"))
            .or_else(|| outputs.first())
            .cloned()
            .expect("sin salidas");
        eprintln!("sonando en: {dev}");
        for name in &outputs {
            if name.contains("VoiceMeeter Input")
                || name.contains("CABLE Input")
                || name.contains("Altavoces")
            {
                match BufferPlayer::play(name, vec![0.0; 1600]) {
                    Ok(_) => eprintln!("OK stream en: {name}"),
                    Err(e) => eprintln!("FALLO stream en {name}: {e:#}"),
                }
            }
        }
        let host = cpal::default_host();
        let device = host
            .output_devices()
            .unwrap()
            .find(|d| device_name(d).as_deref() == Some(&dev))
            .unwrap();
        eprintln!("default_config: {:?}", device.default_output_config());
        match device.supported_output_configs() {
            Ok(cfgs) => {
                for c in cfgs {
                    eprintln!(
                        "soportado: ch={} rate={:?} fmt={:?}",
                        c.channels(),
                        c.min_sample_rate(),
                        c.sample_format()
                    );
                }
            }
            Err(e) => eprintln!("supported err: {e}"),
        }
        let _player = BufferPlayer::play(&dev, vec![0.2; 16_000 * 6]).unwrap();
        let mut hits = 0;
        for _ in 0..10 {
            hits += crate::winvol::unmute_self();
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        eprintln!("session hits: {hits}");
        assert!(hits > 0, "no se halló sesión propia de audio");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_check_devices_and_files() {
        // 1. Dispositivos de audio visibles para la app.
        let (inputs, outputs) = list_devices();
        eprintln!("ENTRADAS: {inputs:?}");
        eprintln!("SALIDAS: {outputs:?}");
        assert!(!inputs.is_empty(), "sin micrófonos visibles");
        assert!(!outputs.is_empty(), "sin salidas visibles");

        // 2. Binarios y modelos locales necesarios.
        let need = [
            "third_party/whisper.cpp/build/bin/Release/whisper-cli.exe",
            "models/ggml-base.bin",
            "models/mt/en-es/encoder.onnx",
            "models/mt/en-es/decoder.onnx",
            "models/mt/en-es/dec0.onnx",
            "models/mt/en-es/dec_past.onnx",
            "models/mt/en-es/tokenizer.json",
            "models/mt/es-en/encoder.onnx",
            "models/mt/es-en/decoder.onnx",
            "models/mt/es-en/dec0.onnx",
            "models/mt/es-en/dec_past.onnx",
            "models/mt/es-en/tokenizer.json",
            "voices/en_US-lessac-medium.onnx",
            "voices/es_ES-sharvard-medium.onnx",
        ];
        let mut missing = Vec::new();
        for f in need {
            if !std::path::Path::new(f).exists() {
                missing.push(f);
            }
        }
        assert!(missing.is_empty(), "faltan archivos: {missing:?}");

        // 3. Cables virtuales (al menos un par para una dirección).
        let has_cable = inputs.iter().any(|d| {
            let l = d.to_lowercase();
            l.contains("cable") || l.contains("voicemeeter") || l.contains("mezcla")
        });
        eprintln!("cable virtual para captura: {has_cable}");
    }
}

fn find_output_by_name(host: &cpal::Host, name: &str) -> anyhow::Result<cpal::Device> {
    host.output_devices()
        .context("sin dispositivos de salida")?
        .find(|d| device_name(d).as_deref() == Some(name))
        .ok_or_else(|| anyhow!("salida no encontrada: {name}"))
}

/// Reproduce un buffer mono a 16kHz (salida de TTS) en cualquier
/// salida: audífonos o mic virtual (Cable-B Input).
/// Remuestrea por interpolación lineal a la tasa del dispositivo.
pub struct BufferPlayer {
    _stream: cpal::Stream,
    pub done: Arc<AtomicBool>,
}

impl BufferPlayer {
    pub fn play(device_name_: &str, samples_16k: Vec<f32>) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = find_output_by_name(&host, device_name_)?;
        let config: cpal::StreamConfig = device.default_output_config()?.into();
        let dev_rate = config.sample_rate as f64;
        let channels = config.channels as usize;

        // Remuestreo previo 16k -> tasa del dispositivo.
        let ratio = dev_rate / 16_000.0;
        let out_len = ((samples_16k.len() as f64 * ratio) as usize).max(1);
        let mut resampled = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let pos = i as f64 / ratio;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = samples_16k.get(idx).copied().unwrap_or(0.0);
            let b = samples_16k.get(idx + 1).copied().unwrap_or(0.0);
            resampled.push(a + (b - a) * frac);
        }

        let data = Arc::new(resampled);
        let pos = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let data_cb = data.clone();
        let pos_cb = pos.clone();
        let done_cb = done.clone();

        let stream = device.build_output_stream(
            config,
            move |out: &mut [f32], _| {
                for frame in out.chunks_mut(channels) {
                    let i = pos_cb.fetch_add(1, Ordering::Relaxed);
                    let s = data_cb.get(i).copied().unwrap_or(0.0);
                    if i >= data_cb.len() && !done_cb.load(Ordering::Relaxed) {
                        done_cb.store(true, Ordering::Relaxed);
                    }
                    for o in frame {
                        *o = s;
                    }
                }
            },
            |e| eprintln!("buffer play error: {e}"),
            None,
        )?;
        stream.play()?;
        // Fuerza volumen 100 sin mute, reintentando mientras aparece la sesión.
        crate::winvol::unmute_self_retry();
        Ok(Self { _stream: stream, done })
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}
