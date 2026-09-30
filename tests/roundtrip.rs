//! Roundtrip real: Piper (voz masculina) -> WAV -> whisper-cli.
//! Se omite si faltan binarios/modelos/voces.

use std::path::PathBuf;

fn exe() -> PathBuf {
    PathBuf::from("third_party/whisper.cpp/build/bin/Release/whisper-cli.exe")
}

fn model() -> PathBuf {
    PathBuf::from("models/ggml-base.bin")
}

fn ready() -> bool {
    exe().exists()
        && model().exists()
        && PathBuf::from("voices/en_US-lessac-medium.onnx").exists()
        && PathBuf::from("voices/es_ES-sharvard-medium.onnx").exists()
}

fn synthesize(text: &str, voice: &str, speaker: Option<&str>, out_wav: &std::path::Path) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut args = vec!["-m", voice, "-f", out_wav.to_str().unwrap()];
    if let Some(s) = speaker {
        args.push("--speaker");
        args.push(s);
    }
    let mut child = Command::new("cmd")
        .args(["/C", "piper"])
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("piper");
    child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "piper: {}", String::from_utf8_lossy(&out.stderr));
}

fn transcribe(wav: &std::path::Path, lang: &str) -> String {
    let out = std::process::Command::new(exe())
        .args([
            "-m", model().to_str().unwrap(),
            "-l", lang,
            "-f", wav.to_str().unwrap(),
            "--no-timestamps", "--no-prints",
            "-t", "8",
        ])
        .output()
        .expect("whisper-cli");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .last()
        .unwrap_or("")
        .to_owned()
}

#[test]
fn en_roundtrip_contains_hello() {
    if !ready() {
        eprintln!("omitido: faltan binarios");
        return;
    }
    let wav = std::env::temp_dir().join("freetouch_rt_en.wav");
    synthesize("Hello, how are you today", "voices/en_US-lessac-medium.onnx", None, &wav);
    let t = transcribe(&wav, "en");
    assert!(t.to_lowercase().contains("hello"), "obtenido: {t}");
}

#[test]
fn es_roundtrip_contains_buenos_dias() {
    if !ready() {
        eprintln!("omitido: faltan binarios");
        return;
    }
    let wav = std::env::temp_dir().join("freetouch_rt_es.wav");
    synthesize(
        "Buenos días, gracias por llamar",
        "voices/es_ES-sharvard-medium.onnx",
        Some("0"),
        &wav,
    );
    let t = transcribe(&wav, "es");
    assert!(t.to_lowercase().contains("buenos"), "obtenido: {t}");
}
