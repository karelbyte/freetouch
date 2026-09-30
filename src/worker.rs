use crate::{audio, mt, stt, tts, vad};
use crate::tts::Synthesizer as _;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{HeapRb, traits::*};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct DirStatus {
    pub running: bool,
    pub phase: String,
    pub last_text: String,
    pub last_trans: String,
    pub phrases: u32,
}

/// Una dirección en vivo, ej: mic_real -> VAD -> STT(ES) -> MT -> TTS(EN) -> Cable-B.
pub struct LiveWorker {
    pub status: Arc<Mutex<DirStatus>>,
    pub log_path: String,
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Log de una iteración Iniciar->Detener: se guarda en
/// %APPDATA%/freetouch/logs para poder compartirlo.
pub struct SessionLog {
    out: Option<BufWriter<File>>,
    pub path: String,
    t0: Instant,
}

impl SessionLog {
    pub fn open(tag: &str, input_name: &str, output_name: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut dir = std::env::var("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        dir.push("freetouch");
        dir.push("logs");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("sesion-{tag}-{now}.txt"));
        let path_s = path.to_string_lossy().to_string();
        let out = File::create(&path)
            .ok()
            .map(BufWriter::new);
        let mut log = Self { out, path: path_s, t0: Instant::now() };
        log.writeln(&format!("sesion {tag} inicio={now}"));
        log.writeln(&format!("ruta: {input_name} -> {output_name}"));
        log
    }

    pub fn elapsed(&self) -> f32 {
        self.t0.elapsed().as_secs_f32()
    }

    pub fn writeln(&mut self, line: &str) {
        let t = self.elapsed();
        if let Some(o) = self.out.as_mut() {
            let _ = writeln!(o, "[{t:>8.1}s] {line}");
            let _ = o.flush();
        }
    }

    pub fn close(mut self, phrases: u32) {
        self.writeln(&format!("fin: {phrases} frases"));
        // Al soltar, BufWriter vuelca el resto.
    }
}

pub struct DirConfig {
    pub tag: String,
    pub input_name: String,
    pub output_name: String,
    pub stt_lang: stt::SttLang,
    pub tts_lang: tts::TtsLang,
    pub mt_dir: mt::MtDir,
    pub use_small_stt: bool,
    pub use_real_mt: bool,
    pub threads: u32,
}

/// Abre la captura y devuelve (stream, extractor del ring, config).
/// El extractor evita nombrar el tipo concreto del consumidor.
fn open_input_stream(
    host: &cpal::Host,
    input_name: &str,
) -> anyhow::Result<(
    cpal::Stream,
    Box<dyn FnMut() -> Option<f32> + Send>,
    cpal::StreamConfig,
)> {
    let in_dev = host
        .input_devices()?
        .find(|d| audio::device_name(d).as_deref() == Some(input_name))
        .ok_or_else(|| anyhow::anyhow!("entrada no encontrada: {input_name}"))?;
    let in_supported = in_dev.default_input_config()?;
    let in_fmt = in_supported.sample_format();
    let in_cfg: cpal::StreamConfig = in_supported.into();
    let in_rate = in_cfg.sample_rate;
    let in_ch = in_cfg.channels as usize;

    // Cola lock-free callback -> worker.
    let rb = HeapRb::<f32>::new((in_rate as usize * in_ch).max(48000) * 10);
    let (mut prod, mut cons) = rb.split();
    let popper: Box<dyn FnMut() -> Option<f32> + Send> =
        Box::new(move || cons.try_pop());

    // push_slice sobrescribe lo más viejo si el worker va lento (STT largo).
    let in_stream = match in_fmt {
        cpal::SampleFormat::F32 => in_dev.build_input_stream(
            in_cfg.clone(),
            move |data: &[f32], _| {
                prod.push_slice(data);
            },
            |e| eprintln!("live capture error: {e}"),
            None,
        )?,
        cpal::SampleFormat::I16 => in_dev.build_input_stream(
            in_cfg.clone(),
            move |data: &[i16], _| {
                let f: Vec<f32> =
                    data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                prod.push_slice(&f);
            },
            |e| eprintln!("live capture error: {e}"),
            None,
        )?,
        cpal::SampleFormat::U16 => in_dev.build_input_stream(
            in_cfg.clone(),
            move |data: &[u16], _| {
                let f: Vec<f32> = data
                    .iter()
                    .map(|s| (*s as f32 - 32768.0) / 32768.0)
                    .collect();
                prod.push_slice(&f);
            },
            |e| eprintln!("live capture error: {e}"),
            None,
        )?,
        f => anyhow::bail!("formato de captura no soportado: {f:?}"),
    };
    Ok((in_stream, popper, in_cfg))
}

/// Hilo dedicado a reproducir: arranca la siguiente frase en cuanto
/// termina la anterior aunque el worker siga ocupado.
fn spawn_pump(
    pending: Arc<Mutex<VecDeque<Vec<f32>>>>,
    run: Arc<AtomicBool>,
    output_name: String,
    st: Arc<Mutex<DirStatus>>,
) {
    let pump_pending = pending;
    let pump_run = run;
    std::thread::spawn(move || {
        let mut player: Option<audio::BufferPlayer> = None;
        while pump_run.load(Ordering::Relaxed) {
            let need_next = match &player {
                None => true,
                Some(p) => p.is_done(),
            };
            if need_next {
                let next = pump_pending
                    .lock()
                    .map(|mut q| q.pop_front())
                    .unwrap_or(None);
                if let Some(samples) = next {
                            match audio::BufferPlayer::play(&output_name, samples) {
                        Ok(p) => {
                            player = Some(p);
                            set_phase(&st, "Hablando…".to_owned());
                            crate::winvol::unmute_self();
                        }
                        Err(e) => {
                            set_phase(&st, format!("Audio err: {e:#}"));
                            std::thread::sleep(std::time::Duration::from_millis(500));
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    });
}

/// Punto de corte para TTS en streaming: fin de frase con >=3 palabras,
/// o corte duro a las 10. Devuelve el fin (exclusivo) del trozo.
fn stream_cut(words: &[String]) -> Option<usize> {
    for (i, w) in words.iter().enumerate().take(14) {
        let t = w.trim_end_matches(['"', '\'', '’', ')', ']', '*']);
        if (t.ends_with('.') || t.ends_with('?') || t.ends_with('!') || t.ends_with('…'))
            && i + 1 >= 3
        {
            return Some(i + 1);
        }
    }
    if words.len() >= 10 {
        return Some(10);
    }
    None
}

impl LiveWorker {
    pub fn start(cfg: DirConfig) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let (in_stream, mut pop, in_cfg) = open_input_stream(&host, &cfg.input_name)?;
        let in_rate = in_cfg.sample_rate;
        let in_ch = in_cfg.channels as usize;

        let status = Arc::new(Mutex::new(DirStatus { running: true, ..Default::default() }));
        let running = Arc::new(AtomicBool::new(true));
        let st = status.clone();
        let run = running.clone();
        let mut seslog = SessionLog::open(&cfg.tag, &cfg.input_name, &cfg.output_name);
        let log_path = seslog.path.clone();

        let handle = std::thread::spawn(move || {
            let mut seg = vad::PhraseSegmenter::new(in_rate, in_ch);
            let mut transcriber: Box<dyn stt::Transcriber + Send> = if cfg.use_small_stt
            {
                // Modelo small vía CLI (preciso pero lento).
                match stt::CliTranscriber::with_defaults() {
                    Ok(mut t) => {
                        t.use_small_model();
                        t.threads = cfg.threads;
                        Box::new(t)
                    }
                    Err(e) => {
                        set_phase(&st, format!("Error STT: {e:#}"));
                        return;
                    }
                }
            } else {
                // Servidor persistente (rápido); si falla, CLI base.
                match stt::ServerStt::launch(cfg.stt_lang, cfg.threads) {
                    Ok(s) => {
                        seslog.writeln("STT: servidor persistente listo");
                        Box::new(s)
                    }
                    Err(e) => {
                        let msg = format!("STT servidor no ({e:#}); usando CLI.");
                        set_phase(&st, msg.clone());
                        seslog.writeln(&msg);
                        match stt::CliTranscriber::with_defaults() {
                            Ok(mut t) => {
                                t.threads = cfg.threads;
                                Box::new(t)
                            }
                            Err(e2) => {
                                set_phase(&st, format!("Error STT: {e2:#}"));
                                return;
                            }
                        }
                    }
                }
            };
            let mut translator: Box<dyn mt::Translator + Send> = if cfg.use_real_mt {
                match mt::RealTranslator::with_defaults(4) {
                    Ok(r) => Box::new(r),
                    Err(e) => {
                        set_phase(&st, format!("Error MT real ({e:#}); usando mock."));
                        Box::new(mt::MockTranslator)
                    }
                }
            } else {
                Box::new(mt::MockTranslator)
            };
            let mut synth = match tts::PiperSynthesizer::with_defaults() {
                Ok(s) => s,
                Err(e) => {
                    set_phase(&st, format!("Error TTS: {e:#}"));
                    return;
                }
            };
            let pending: Arc<Mutex<VecDeque<Vec<f32>>>> =
                Arc::new(Mutex::new(VecDeque::new()));
            spawn_pump(pending.clone(), run.clone(), cfg.output_name.clone(), st.clone());

            if let Err(e) = in_stream.play() {
                set_phase(&st, format!("Error captura: {e:#}"));
                return;
            }
            let _keep_stream = in_stream;
            set_phase(&st, "Escuchando…".to_owned());

            let mut buf = Vec::with_capacity(48000);
            let mut tick: u32 = 0;
            while run.load(Ordering::Relaxed) {
                tick = tick.wrapping_add(1);
                // Windows recuerda el volumen 1 y lo reimpone: reforzarlo
                // periódicamente mientras hay audio por sonar.
                let queued = pending.lock().map(|q| !q.is_empty()).unwrap_or(false);
                if queued && tick % 10 == 0 {
                    crate::winvol::unmute_self();
                }
                buf.clear();
                while let Some(s) = pop() {
                    buf.push(s);
                    if buf.len() >= 48000 {
                        break;
                    }
                }
                if !buf.is_empty() {
                    let emitted = seg.push(&buf);
                    for phrase in emitted {
                        set_phase(&st, "Transcribiendo…".to_owned());
                        let t_stt = Instant::now();
                        let text = match transcriber.transcribe(&phrase.samples_16k, cfg.stt_lang)
                        {
                            Ok(r) => r.text,
                            Err(e) => {
                                set_phase(&st, format!("STT err: {e:#}"));
                                seslog.writeln(&format!("STT err: {e:#}"));
                                continue;
                            }
                        };
                        let d_stt = t_stt.elapsed();
                        if text.trim().is_empty() {
                            seslog.writeln(&format!(
                                "STT vacio ({:.1}s audio, {}ms)",
                                phrase.samples_16k.len() as f32 / 16000.0,
                                d_stt.as_millis()
                            ));
                            continue;
                        }
                        set_phase(&st, "Traduciendo…".to_owned());
                        let t_mt = Instant::now();
                        let trans = match translator.translate(&text, cfg.mt_dir) {
                            Ok(r) => r.text,
                            Err(e) => {
                                set_phase(&st, format!("MT err: {e:#}"));
                                seslog.writeln(&format!("MT err: {e:#}"));
                                continue;
                            }
                        };
                        let d_mt = t_mt.elapsed();
                        set_phase(&st, "Sintetizando voz…".to_owned());
                        let tts_lang = cfg.tts_lang;
                        let t_tts = Instant::now();
                        match synth.synthesize(&trans, tts_lang) {
                            Ok(a) => {
                                let d_tts = t_tts.elapsed();
                                if let Ok(mut q) = pending.lock() {
                                    q.push_back(a.samples_16k);
                                }
                                let mut s = st.lock().unwrap();
                                s.last_text = text.clone();
                                s.last_trans = trans.clone();
                                s.phrases += 1;
                                seslog.writeln(&format!(
                                    "dijo: {} || trad: {} || stt={}ms mt={}ms tts={}ms",
                                    text.trim(),
                                    trans.trim(),
                                    d_stt.as_millis(),
                                    d_mt.as_millis(),
                                    d_tts.as_millis()
                                ));
                            }
                            Err(e) => {
                                set_phase(&st, format!("TTS err: {e:#}"));
                                seslog.writeln(&format!("TTS err: {e:#}"));
                            }
                        }
                    }
                }
                // El bombeo lo hace el hilo dedicado; aquí solo se vuelve
                // a escucha cuando no hay nada pendiente ni hablando.
                let idle = pending.lock().map(|q| q.is_empty()).unwrap_or(true);
                if idle {
                    if seg.is_speaking() {
                        set_phase(&st, "Escuchando…".to_owned());
                    } else if !st.lock().unwrap().phase.starts_with("Escuch")
                        && !st.lock().unwrap().phase.starts_with("Habla")
                    {
                        // Mantiene última fase útil.
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            let n = st.lock().map(|s| s.phrases).unwrap_or(0);
            seslog.close(n);
            set_phase(&st, "Detenido.".to_owned());
        });

        Ok(Self { status, running, handle: Some(handle), log_path })
    }

    /// Modo streaming (experimental): ventanas STT + retraducción MT +
    /// TTS por trozos con corte en puntuación. Objetivo ~2s de retardo.
    pub fn start_stream(cfg: DirConfig) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let (in_stream, mut pop, in_cfg) = open_input_stream(&host, &cfg.input_name)?;
        let in_rate = in_cfg.sample_rate;
        let in_ch = in_cfg.channels as usize;

        let status = Arc::new(Mutex::new(DirStatus { running: true, ..Default::default() }));
        let running = Arc::new(AtomicBool::new(true));
        let st = status.clone();
        let run = running.clone();
        let mut seslog =
            SessionLog::open(&format!("{}-S", cfg.tag), &cfg.input_name, &cfg.output_name);
        let log_path = seslog.path.clone();

        let pending: Arc<Mutex<VecDeque<Vec<f32>>>> = Arc::new(Mutex::new(VecDeque::new()));
        spawn_pump(pending.clone(), run.clone(), cfg.output_name.clone(), st.clone());

        if let Err(e) = in_stream.play() {
            set_phase(&st, format!("Error captura: {e:#}"));
            return Err(anyhow::anyhow!("captura: {e:#}"));
        }
        set_phase(&st, "Cargando…".to_owned());

        let handle = std::thread::spawn(move || {
            // OJO: el stream debe vivir DENTRO del hilo; si se queda fuera
            // se destruye al salir de start_stream y no entra audio.
            let _keep_stream = in_stream;
            set_phase(&st, "Cargando STT…".to_owned());
            let mut stt = match stt::SttStream::launch(cfg.stt_lang, 4.0, 0.5) {
                Ok(s) => {
                    seslog.writeln("STT streaming listo");
                    s
                }
                Err(e) => {
                    seslog.writeln(&format!("STT stream no ({e:#})"));
                    set_phase(&st, format!("Error STT stream: {e:#}"));
                    return;
                }
            };
            set_phase(&st, "Cargando MT…".to_owned());
            let translator: Box<dyn mt::Translator + Send> = if cfg.use_real_mt {
                match mt::RealTranslator::with_defaults(4) {
                    Ok(r) => Box::new(r),
                    Err(e) => {
                        set_phase(&st, format!("Error MT real ({e:#}); usando mock."));
                        Box::new(mt::MockTranslator)
                    }
                }
            } else {
                Box::new(mt::MockTranslator)
            };
            let mut mts = mt::MtStream::new(translator, cfg.mt_dir, 3);
            set_phase(&st, "Cargando TTS…".to_owned());
            let mut synth = match tts::PiperSynthesizer::with_defaults() {
                Ok(s) => s,
                Err(e) => {
                    set_phase(&st, format!("Error TTS: {e:#}"));
                    seslog.writeln(&format!("TTS no: {e:#}"));
                    return;
                }
            };
            set_phase(&st, "Escuchando…".to_owned());
            let mut resampler = vad::Resampler16k::new(in_rate, in_ch);
            let mut chunk16k = Vec::new();
            let mut buf = Vec::with_capacity(48000);
            let mut tgt_buf: Vec<String> = Vec::new();
            let mut tick: u32 = 0;
            let mut last_cut_tick: u32 = 0;
            while run.load(Ordering::Relaxed) {
                tick = tick.wrapping_add(1);
                buf.clear();
                while let Some(s) = pop() {
                    buf.push(s);
                    if buf.len() >= 48000 {
                        break;
                    }
                }
                if !buf.is_empty() {
                    chunk16k.clear();
                    resampler.push_interleaved(&buf, &mut chunk16k);
                    // Palabras fuente estables -> retraducir.
                    let mut fresh_src = Vec::new();
                    for w in stt.push(&chunk16k) {
                        fresh_src.push(w);
                    }
                    if !fresh_src.is_empty() {
                        for t in mts.push(&fresh_src) {
                            tgt_buf.push(t);
                        }
                    }
                    // Corta trozo en puntuación (>=3 palabras) o a las 10.
                    // Si hay palabras atoradas >=2s sin corte, fuerza el corte.
                    loop {
                        let end = match stream_cut(&tgt_buf) {
                            Some(e) => Some((e, false)),
                            None
                                if tgt_buf.len() >= 3
                                    && tick.wrapping_sub(last_cut_tick) > 65 =>
                            {
                                Some((tgt_buf.len(), true))
                            }
                            None => None,
                        };
                        let Some((end, forced)) = end else {
                            break;
                        };
                        last_cut_tick = tick;
                        let piece: Vec<String> = tgt_buf.drain(0..end).collect();
                        let text = piece.join(" ");
                        let t_tts = Instant::now();
                        match synth.synthesize(&text, cfg.tts_lang) {
                            Ok(a) => {
                                let d_tts = t_tts.elapsed();
                                if let Ok(mut q) = pending.lock() {
                                    q.push_back(a.samples_16k);
                                }
                                let mut s = st.lock().unwrap();
                                s.last_text = format!("~{}", tgt_buf.len());
                                s.last_trans = text.clone();
                                s.phrases += 1;
                                seslog.writeln(&format!(
                                    "stream{}: {} || tts={}ms (stt~{}ms mt~{}ms)",
                                    if forced { "(force)" } else { "" },
                                    text.trim(),
                                    d_tts.as_millis(),
                                    stt.last_ms,
                                    mts.last_ms
                                ));
                            }
                            Err(e) => {
                                set_phase(&st, format!("TTS err: {e:#}"));
                                seslog.writeln(&format!("TTS err: {e:#}"));
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            // Vacía la cola al detener.
            for w in stt.flush() {
                for t in mts.push(&[w]) {
                    tgt_buf.push(t);
                }
            }
            for t in mts.flush() {
                tgt_buf.push(t);
            }
            if !tgt_buf.is_empty() {
                let text = tgt_buf.join(" ");
                if let Ok(a) = synth.synthesize(&text, cfg.tts_lang) {
                    if let Ok(mut q) = pending.lock() {
                        q.push_back(a.samples_16k);
                    }
                    seslog.writeln(&format!("stream-final: {}", text.trim()));
                }
            }
            let n = st.lock().map(|s| s.phrases).unwrap_or(0);
            seslog.close(n);
            set_phase(&st, "Detenido.".to_owned());
        });

        Ok(Self { status, running, handle: Some(handle), log_path })
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        if let Ok(mut s) = self.status.lock() {
            s.running = false;
        }
    }
}

impl Drop for LiveWorker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

fn set_phase(st: &Arc<Mutex<DirStatus>>, phase: String) {
    if let Ok(mut s) = st.lock() {
        s.phase = phase;
    }
}
