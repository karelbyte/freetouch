//! QA del cable virtual por loopback real (usa el Cable B para no
//! interferir con pruebas en curso sobre el Cable A):
//! genera un tono de 440Hz en "Altavoces (FreeTouch Cable B)", lo graba
//! desde "Micrófono (FreeTouch Cable B)" y verifica nivel y tono.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

fn find_output(host: &cpal::Host,frag: &str) -> cpal::Device {
    host.output_devices()
        .unwrap()
        .find(|d| {
            d.description()
                .map(|x| x.name().contains(frag))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("sin salida {frag}"))
}

/// Fuerza volumen 100 sin mute en NUESTRAS sesiones (Windows deja las
/// sesiones nuevas en 1/mute; es lo mismo que hace winvol::unmute_self).
fn unmute_own_sessions() {
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceCollection,
        IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eRender,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::core::Interface;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return;
        };
        let Ok(collection): Result<IMMDeviceCollection, _> =
            enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
        else {
            return;
        };
        let ndev = collection.GetCount().unwrap_or(0);
        let pid = std::process::id();
        for d in 0..ndev {
            let Ok(device) = collection.Item(d) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
            for i in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(ctl) = sessions.GetSession(i) else {
                    continue;
                };
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                if ctl2.GetProcessId().unwrap_or(0) != pid {
                    continue;
                }
                if let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() {
                    let _ = vol.SetMasterVolume(1.0, std::ptr::null());
                    let _ = vol.SetMute(false, std::ptr::null());
                }
            }
        }
    }
}

fn find_input(host: &cpal::Host, frag: &str) -> cpal::Device {
    host.input_devices()
        .unwrap()
        .find(|d| {
            d.description()
                .map(|x| x.name().contains(frag))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("sin entrada {frag}"))
}

/// Control: mismo harness contra Realtek (altavoces + mezcla estéreo).
/// Si esto pasa y el cable falla, el problema es el cable; si esto
/// también falla, el problema es el harness.
#[test]
#[ignore = "requiere audio exclusivo y volumen manual; las sesiones nuevas arrancan muteadas"]
fn control_realtek_loopback_tone() {
    loopback_tone_check(
        "Altavoces (Realtek(R) Audio)",
        "Mezcla estéreo (Realtek(R) Audio)",
        0.05,
    );
}

#[test]
#[ignore = "requiere audio exclusivo y volumen manual; las sesiones nuevas arrancan muteadas"]
fn cable_b_loopback_tone_level_and_pitch() {
    loopback_tone_check(
        "Altavoces (FreeTouch Cable B)",
        "Micrófono (FreeTouch Cable B)",
        0.5,
    );
}

fn loopback_tone_check(out_frag: &str, in_frag: &str, amp: f32) {
    let host = cpal::default_host();
    let out_dev = find_output(&host, out_frag);
    let in_dev = find_input(&host, in_frag);

    // Tono 440Hz estéreo 48k, 3 segundos.
    let rate = 48_000u32;
    let tone: Vec<f32> = (0..rate as usize * 3)
        .map(|i| {
            (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / rate as f32).sin() * amp
        })
        .collect();

    // Graba el micro en segundo plano.
    let in_cfg: cpal::StreamConfig = in_dev.default_input_config().unwrap().into();
    let in_rate = in_cfg.sample_rate;
    let captured: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let cap_cb = captured.clone();
    let in_stream = in_dev
        .build_input_stream(
            in_cfg,
            move |data: &[f32], _| {
                cap_cb.lock().unwrap().extend_from_slice(data);
            },
            |e| eprintln!("cap err: {e}"),
            None,
        )
        .unwrap();
    in_stream.play().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Reproduce el tono por el render del cable.
    let out_cfg: cpal::StreamConfig = out_dev.default_output_config().unwrap().into();
    let channels = out_cfg.channels as usize;
    let pos = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pos_cb = pos.clone();
    let tone_arc = Arc::new(tone);
    let tone_cb = tone_arc.clone();
    let out_stream = out_dev
        .build_output_stream(
            out_cfg,
            move |out: &mut [f32], _| {
                for frame in out.chunks_mut(channels) {
                    let i = pos_cb.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // Mezcla a mono: promedio de L/R del tono (idénticos).
                    let s = tone_cb.get(i).copied().unwrap_or(0.0);
                    for o in frame {
                        *o = s;
                    }
                }
            },
            |e| eprintln!("play err: {e}"),
            None,
        )
        .unwrap();
    out_stream.play().unwrap();
    // La sesion tarda en aparecer: fuerza volumen varias veces.
    for _ in 0..6 {
        unmute_own_sessions();
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    drop(out_stream);
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(in_stream);

    let data = captured.lock().unwrap().clone();
    eprintln!("muestras capturadas: {} a {}Hz", data.len(), in_rate);
    assert!(data.len() > in_rate as usize, "casi no llegó audio");

    // Nivel RMS en la zona central (evita transitorios).
    let mid = &data[data.len() / 4..data.len() * 3 / 4];
    // A mono: promedia canales si hace falta (supone intercalado).
    let nch = 2usize.min(4);
    let mut mono = Vec::with_capacity(mid.len() / nch);
    for fr in mid.chunks(nch) {
        mono.push(fr.iter().sum::<f32>() / fr.len() as f32);
    }
    let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
    eprintln!("RMS={rms:.3}");
    assert!(rms > amp * 0.25, "nivel muy bajo, RMS={rms}");

    // Energía por bandas (Goertzel) para ver qué hay además del tono.
    fn goertzel(x: &[f32], rate: f32, target: f32) -> f32 {
        let n = x.len() as f32;
        let k = 0.5 + target * n / rate;
        let w = 2.0 * std::f32::consts::PI * k / n;
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &v in x {
            let s0 = v + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).sqrt() / n
    }
    let total = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
    for f in [440.0, 880.0, 1760.0, 3000.0, 5000.0, 7500.0] {
        let e = goertzel(&mono, in_rate as f32, f);
        eprintln!("banda {f:>6.0}Hz: {e:.4} ({:.0}% del total)", 100.0 * e / total);
    }
    let e440 = goertzel(&mono, in_rate as f32, 440.0);
    assert!(
        e440 / total > 0.5,
        "el tono 440Hz no domina (posible contaminacion o distorsion)"
    );
}
