mod audio;
mod mt;
mod mt_onnx;
mod stt;
mod tts;
mod vad;
mod wav;
mod winvol;
mod worker;

use eframe::egui;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SavedConfig {
    mic_real: String,
    far_capture: String,
    headphones: String,
    virt_mic_play: String,
    use_small_stt: bool,
    use_streaming: bool,
}

fn config_path() -> std::path::PathBuf {
    if let Ok(test) = std::env::var("FREETOUCH_CONFIG") {
        return std::path::PathBuf::from(test);
    }
    let base = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    base.join("freetouch").join("config.json")
}

fn load_config() -> SavedConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

struct FreeTouchApp {
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    mic_real: String,
    far_capture: String,
    headphones: String,
    virt_mic_play: String,
    log: Vec<String>,
    live_out: Option<worker::LiveWorker>,
    live_in: Option<worker::LiveWorker>,
    use_small_stt: bool,
    use_streaming: bool,
    use_real_mt: bool,
    last_saved: SavedConfig,
}

impl FreeTouchApp {
    fn new() -> Self {
        let (inputs, outputs) = audio::list_devices();
        let saved = load_config();
        // Solo restaura lo que siga existiendo.
        let pick = |want: &str, list: &[String]| -> String {
            if !want.is_empty() && list.iter().any(|d| d == want) {
                want.to_owned()
            } else {
                String::new()
            }
        };
        let mic_real = {
            let m = pick(&saved.mic_real, &inputs);
            if m.is_empty() {
                inputs.first().cloned().unwrap_or_default()
            } else {
                m
            }
        };
        let headphones = {
            let h = pick(&saved.headphones, &outputs);
            if h.is_empty() {
                outputs.first().cloned().unwrap_or_default()
            } else {
                h
            }
        };
        let restored = !saved.mic_real.is_empty() || !saved.headphones.is_empty();
        let mut app = Self {
            mic_real,
            far_capture: pick(&saved.far_capture, &inputs),
            headphones,
            virt_mic_play: pick(&saved.virt_mic_play, &outputs),
            input_devices: inputs,
            output_devices: outputs,
            log: vec!["Elige tus dispositivos y pulsa Iniciar.".to_owned()],
            live_out: None,
            live_in: None,
            use_small_stt: saved.use_small_stt,
            use_streaming: saved.use_streaming,
            use_real_mt: true,
            last_saved: SavedConfig::default(),
        };
        app.last_saved = app.current_config();
        if restored {
            app.push_log("Aparatos restaurados de la última vez.".to_owned());
        }
        app
    }

    fn current_config(&self) -> SavedConfig {
        SavedConfig {
            mic_real: self.mic_real.clone(),
            far_capture: self.far_capture.clone(),
            headphones: self.headphones.clone(),
            virt_mic_play: self.virt_mic_play.clone(),
            use_small_stt: self.use_small_stt,
            use_streaming: self.use_streaming,
        }
    }

    /// Guarda si algo cambió desde la última vez.
    fn maybe_save(&mut self) {
        let cur = self.current_config();
        if cur.mic_real != self.last_saved.mic_real
            || cur.far_capture != self.last_saved.far_capture
            || cur.headphones != self.last_saved.headphones
            || cur.virt_mic_play != self.last_saved.virt_mic_play
            || cur.use_small_stt != self.last_saved.use_small_stt
            || cur.use_streaming != self.last_saved.use_streaming
        {
            let path = config_path();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&path, serde_json::to_string_pretty(&cur).unwrap_or_default())
                .is_ok()
            {
                self.last_saved = cur;
            }
        }
    }

    fn refresh_devices(&mut self) {
        let (inputs, outputs) = audio::list_devices();
        self.input_devices = inputs;
        self.output_devices = outputs;
        // Recupera lo guardado para los huecos vacíos o desconectados.
        let saved = load_config();
        let mut recovered = Vec::new();
        let mut fix = |slot: &mut String, want: &str, list: &[String], label: &'static str| {
            if (slot.is_empty() || !list.iter().any(|d| d == slot)) && !want.is_empty() {
                if list.iter().any(|d| d == want) {
                    *slot = want.to_owned();
                    recovered.push(label);
                }
            }
        };
        fix(&mut self.mic_real, &saved.mic_real, &self.input_devices, "micrófono");
        fix(
            &mut self.far_capture,
            &saved.far_capture,
            &self.input_devices,
            "ellos",
        );
        fix(
            &mut self.headphones,
            &saved.headphones,
            &self.output_devices,
            "audífonos",
        );
        fix(
            &mut self.virt_mic_play,
            &saved.virt_mic_play,
            &self.output_devices,
            "mic virtual",
        );
        if recovered.is_empty() {
            self.push_log("Dispositivos actualizados.".to_owned());
        } else {
            self.push_log(format!("Recuperado: {}.", recovered.join(", ")));
        }
    }

    fn push_log(&mut self, msg: String) {
        self.log.push(msg);
        if self.log.len() > 100 {
            let excess = self.log.len() - 100;
            self.log.drain(0..excess);
        }
    }
}

fn live_row(
    ui: &mut egui::Ui,
    label: &str,
    route: &str,
    status: Option<std::sync::Arc<std::sync::Mutex<worker::DirStatus>>>,
    starter: impl FnOnce() -> Result<worker::LiveWorker, String>,
    slot: &mut Option<worker::LiveWorker>,
    log: &mut Vec<String>,
) {
    ui.label(format!("{label}: {route}"));
    if let Some(st) = &status {
        if let Ok(s) = st.lock() {
            ui.label(format!("  [{}] frases: {}", s.phase, s.phrases));
            if !s.last_text.is_empty() {
                ui.label(format!("  dijo: {}", s.last_text));
                ui.label(format!("  trad: {}", s.last_trans));
            }
        }
    }
    let running = slot.is_some();
    if ui
        .button(if running { "■ Detener" } else { "▶ Iniciar" })
        .clicked()
    {
        if running {
            if let Some(mut w) = slot.take() {
                let p = w.log_path.clone();
                w.stop();
                if p.is_empty() {
                    log.push(format!("{label} detenido."));
                } else {
                    log.push(format!("{label} detenido. Sesión: {p}"));
                }
            }
        } else {
            match starter() {
                Ok(w) => {
                    *slot = Some(w);
                    let p = slot
                        .as_ref()
                        .map(|x| x.log_path.clone())
                        .unwrap_or_default();
                    if p.is_empty() {
                        log.push(format!("{label} iniciado."));
                    } else {
                        log.push(format!("{label} iniciado. Sesión: {p}"));
                    }
                }
                Err(e) => log.push(format!("{label}: {e}")),
            }
        }
    }
}

impl eframe::App for FreeTouchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.style_mut().spacing.item_spacing = egui::vec2(6.0, 8.0);
        ui.style_mut().spacing.button_padding = egui::vec2(10.0, 6.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        self.ui_body(ui);
                    });
                    ui.add_space(10.0);
                });
                ui.add_space(10.0);
            });
    }
}

impl FreeTouchApp {
    fn ui_body(&mut self, ui: &mut egui::Ui) {
        ui.heading("FreeTouch Traductor");
        ui.label("Hablas español, te oyen en inglés. Te hablan inglés, oyes español.");
        ui.separator();

        ui.label("PASO 1 · Elige tus aparatos:");
        egui::ComboBox::from_label("Tu micrófono")
            .selected_text(&self.mic_real)
            .show_ui(ui, |ui| {
                for d in &self.input_devices.clone() {
                    ui.selectable_value(&mut self.mic_real, d.clone(), d);
                }
            });
        egui::ComboBox::from_label("Tus audífonos")
            .selected_text(&self.headphones)
            .show_ui(ui, |ui| {
                for d in &self.output_devices.clone() {
                    ui.selectable_value(&mut self.headphones, d.clone(), d);
                }
            });
        if ui.button("Actualizar aparatos").clicked() {
            self.refresh_devices();
        }

        ui.separator();
        ui.label("PASO 2 · Elige los cables de la llamada:");
        ui.label("(En Zoom/Meet/WhatsApp pon altavoz = Cable-A y micrófono = Cable-B)");
        egui::ComboBox::from_label("Lo que dicen ellos (Cable-A)")
            .selected_text(if self.far_capture.is_empty() {
                "— elegir —"
            } else {
                &self.far_capture
            })
            .show_ui(ui, |ui| {
                for d in &self.input_devices.clone() {
                    ui.selectable_value(&mut self.far_capture, d.clone(), d);
                }
            });
        egui::ComboBox::from_label("Tu voz hacia ellos (Cable-B)")
            .selected_text(if self.virt_mic_play.is_empty() {
                "— elegir —"
            } else {
                &self.virt_mic_play
            })
            .show_ui(ui, |ui| {
                for d in &self.output_devices.clone() {
                    ui.selectable_value(&mut self.virt_mic_play, d.clone(), d);
                }
            });

        ui.separator();
        ui.label("PASO 3 · Inicia la traducción:");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.use_small_stt, "Más preciso (lento)");
            ui.checkbox(&mut self.use_streaming, "Streaming experimental (~2s)");
        });
        live_row(
            ui,
            "TÚ · español → inglés",
            &format!("{} -> {}", self.mic_real, self.virt_mic_play),
            self.live_out.as_ref().map(|w| w.status.clone()),
            || {
                if self.mic_real.is_empty() || self.virt_mic_play.is_empty() {
                    return Err("Elige tu micrófono y el Cable-B.".to_owned());
                }
                let cfg = worker::DirConfig {
                    tag: "TU".to_owned(),
                    input_name: self.mic_real.clone(),
                    output_name: self.virt_mic_play.clone(),
                    stt_lang: stt::SttLang::Es,
                    tts_lang: tts::TtsLang::En,
                    mt_dir: mt::MtDir::EsToEn,
                    use_small_stt: self.use_small_stt,
                    use_real_mt: self.use_real_mt,
                    threads: 8,
                };
                let r = if self.use_streaming {
                    worker::LiveWorker::start_stream(cfg)
                } else {
                    worker::LiveWorker::start(cfg)
                };
                r.map_err(|e| format!("{e:#}"))
            },
            &mut self.live_out,
            &mut self.log,
        );
        ui.separator();
        live_row(
            ui,
            "ELLOS · inglés → español",
            &format!("{} -> {}", self.far_capture, self.headphones),
            self.live_in.as_ref().map(|w| w.status.clone()),
            || {
                if self.far_capture.is_empty() || self.headphones.is_empty() {
                    return Err("Elige el Cable-A y tus audífonos.".to_owned());
                }
                let cfg = worker::DirConfig {
                    tag: "ELLOS".to_owned(),
                    input_name: self.far_capture.clone(),
                    output_name: self.headphones.clone(),
                    stt_lang: stt::SttLang::En,
                    tts_lang: tts::TtsLang::Es,
                    mt_dir: mt::MtDir::EnToEs,
                    use_small_stt: self.use_small_stt,
                    use_real_mt: self.use_real_mt,
                    threads: 8,
                };
                let r = if self.use_streaming {
                    worker::LiveWorker::start_stream(cfg)
                } else {
                    worker::LiveWorker::start(cfg)
                };
                r.map_err(|e| format!("{e:#}"))
            },
            &mut self.live_in,
            &mut self.log,
        );

        self.maybe_save();

        ui.separator();
        ui.label("Estado:");
        egui::ScrollArea::vertical()
            .max_height(100.0)
            .show(ui, |ui| {
                for line in self.log.iter().rev().take(30) {
                    ui.label(line);
                }
            });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_always_on_top()
            .with_inner_size([440.0, 640.0])
            .with_title("FreeTouch Traductor"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "freetouch",
        options,
        Box::new(|_| Ok(Box::new(FreeTouchApp::new()))),
    )
}
