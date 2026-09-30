//! Fuerza el volumen de nuestras propias sesiones de audio al 100% sin mute.
//! Windows a veces deja la fila de la app en 1 y no deja moverla cuando no
//! hay sesión activa; al llamar esto antes de reproducir se habilita.

#[cfg(windows)]
pub fn unmute_self() -> usize {
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
        let enumerator: Result<IMMDeviceEnumerator, _> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL);
        let enumerator = match enumerator {
            Ok(e) => e,
            Err(_) => return 0,
        };
        // Recorre TODAS las salidas activas (el rol por defecto puede no
        // ser donde suena nuestro stream).
        let collection: IMMDeviceCollection = match enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
            Ok(c) => c,
            Err(_) => return 0,
        };
        let ndev = collection.GetCount().unwrap_or(0);
        let pid = std::process::id();
        let mut hits = 0;
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
            let count = sessions.GetCount().unwrap_or(0);
            for i in 0..count {
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
                    hits += 1;
                }
            }
        }
        hits
    }
}

/// Repite el desbloqueo durante ~2s en segundo plano, porque la sesión
/// de audio tarda un momento en aparecer ante Windows.
#[cfg(windows)]
pub fn unmute_self_retry() {
    std::thread::spawn(|| {
        for (i, ms) in [200u64, 600, 1200].into_iter().enumerate() {
            std::thread::sleep(std::time::Duration::from_millis(
                if i == 0 { ms } else { ms - [200, 600, 1200][i - 1] },
            ));
            unmute_self();
            let _ = i;
        }
    });
}

#[cfg(not(windows))]
pub fn unmute_self_retry() {}

#[cfg(not(windows))]
pub fn unmute_self() -> usize {
    0
}

/// Volcado de volumen/mute de cada salida (diagnóstico, solo tests).
#[cfg(all(windows, test))]
pub fn dump_endpoints() -> Vec<(String, f32, bool)> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
    };

    let mut out = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return out;
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return out;
        };
        let ndev = collection.GetCount().unwrap_or(0);
        for d in 0..ndev {
            let Ok(device) = collection.Item(d) else {
                continue;
            };
            let name = device
                .OpenPropertyStore(STGM_READ)
                .ok()
                .and_then(|st| st.GetValue(&PKEY_Device_FriendlyName).ok())
                .map(|v| format!("{:?}", v))
                .unwrap_or_default();
            let (mut level, mut mute) = (0.0, false);
            if let Ok(ev) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                level = ev.GetMasterVolumeLevelScalar().unwrap_or(0.0);
                mute = ev.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            }
            out.push((name, level, mute));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_endpoints() {
        for (name, level, mute) in super::dump_endpoints() {
            eprintln!("OUT name={name} vol={level:.2} mute={mute}");
        }
    }

    /// Intenta GetMixFormat en cada render FreeTouch (activos o no) para
    /// ver qué dice el motor de cada uno.
    #[test]
    fn probe_render_mixformat() {
        use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        use windows::Win32::Media::Audio::{
            DEVICE_STATE, DEVICE_STATEMASK_ALL, IAudioClient, IMMDeviceEnumerator,
            MMDeviceEnumerator, eRender,
        };
        use windows::Win32::System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
        };
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(enumerator) =
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            else {
                return;
            };
            let Ok(collection) =
                enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE(DEVICE_STATEMASK_ALL))
            else {
                return;
            };
            for d in 0..collection.GetCount().unwrap_or(0) {
                let Ok(device) = collection.Item(d) else {
                    continue;
                };
                let name = device
                    .OpenPropertyStore(STGM_READ)
                    .ok()
                    .and_then(|st| st.GetValue(&PKEY_Device_FriendlyName).ok())
                    .map(|v| format!("{:?}", v))
                    .unwrap_or_default();
                if !name.contains("FreeTouch") {
                    continue;
                }
                let state = device.GetState().map(|s| s.0).unwrap_or(0);
                match device.Activate::<IAudioClient>(CLSCTX_ALL, None) {
                    Ok(client) => match client.GetMixFormat() {
                        Ok(pwfx) => {
                            let (ch, rate, bits) = (
                                std::ptr::addr_of!((*pwfx).nChannels).read_unaligned(),
                                std::ptr::addr_of!((*pwfx).nSamplesPerSec).read_unaligned(),
                                std::ptr::addr_of!((*pwfx).wBitsPerSample).read_unaligned(),
                            );
                            eprintln!(
                                "MIX state={state} ch={ch} rate={rate} bits={bits} name={name}"
                            );
                        }
                        Err(e) => eprintln!("MIX state={state} GetMixFormat err={e:?} name={name}"),
                    },
                    Err(e) => eprintln!("MIX state={state} Activate err={e:?} name={name}"),
                }
            }
        }
    }

    /// Lista endpoints en TODOS los estados (activos, deshabilitados,
    /// no presentes, desconectados), render + captura.
    #[test]
    fn probe_endpoints_all() {
        use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
            use windows::Win32::Media::Audio::{
                DEVICE_STATE, DEVICE_STATEMASK_ALL, IMMDeviceEnumerator, MMDeviceEnumerator,
                eCapture, eRender,
            };
        use windows::Win32::System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
        };
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(enumerator) =
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            else {
                return;
            };
            for (flow, tag) in [(eRender, "RENDER"), (eCapture, "CAPTURE")] {
                let Ok(collection) =
                    enumerator.EnumAudioEndpoints(flow, DEVICE_STATE(DEVICE_STATEMASK_ALL))
                else {
                    continue;
                };
                let ndev = collection.GetCount().unwrap_or(0);
                eprintln!("FLOW {tag} count={ndev}");
                for d in 0..ndev {
                    let Ok(device) = collection.Item(d) else {
                        continue;
                    };
                    let state = device.GetState().map(|s| s.0).unwrap_or(0);
                    let name = device
                        .OpenPropertyStore(STGM_READ)
                        .ok()
                        .and_then(|st| st.GetValue(&PKEY_Device_FriendlyName).ok())
                        .map(|v| format!("{:?}", v))
                        .unwrap_or_default();
                    if name.contains("FreeTouch")
                        || name.contains("FreeT")
                        || state != 1
                    {
                        let id = device.GetId().map(|s| s.to_string().unwrap_or_default()).unwrap_or_default();
                        eprintln!("ALL {tag} state={state} id={id} name={name}");
                    }
                }
            }
        }
    }
}
