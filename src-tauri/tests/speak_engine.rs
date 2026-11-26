//! Reading a reply aloud, end to end.
//!
//! Ignored because it downloads Piper (about 22 MB from GitHub) and one voice
//! (about 63 MB from Hugging Face). It exists because a green build says nothing
//! about the things that actually break: whether the zip unpacks into the layout
//! `engine.rs` expects, whether Piper starts as a child process at all, and
//! whether our own WAV reader can read what Piper writes.
//!
//! Run it with a real network:
//!
//! ```powershell
//! cargo test --test speak_engine --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture
//! ```

use std::sync::{Arc, Mutex};

use novacode_lib::speak::catalog::{self, Source, RUNTIME};
use novacode_lib::speak::engine::SpeakEngine;
use novacode_lib::voice::download::{self, phase, Progress};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio_util::sync::CancellationToken;

/// A channel that records every frame instead of forwarding it to a webview.
fn recorder() -> (Channel<Progress>, Arc<Mutex<Vec<Progress>>>) {
    let frames: Arc<Mutex<Vec<Progress>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&frames);
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(text) = body {
            // Read the frame the way the page does: the JSON, not the Rust type.
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                let received = value["received"].as_u64().unwrap_or(0);
                let total = value["total"].as_u64().unwrap_or(0);
                let phase = match value["phase"].as_str().unwrap_or_default() {
                    "runtime" => "runtime",
                    "voice" => "voice",
                    _ => "model",
                };
                sink.lock().expect("frames mutex").push(Progress {
                    received,
                    total,
                    phase,
                });
            }
        }
        Ok(())
    });
    (channel, frames)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads the runtime and a voice from two hosts"]
async fn downloads_piper_and_a_voice_then_speaks() {
    let root = std::env::temp_dir().join(format!("nova-speak-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (channel, frames) = recorder();
    let cancel = CancellationToken::new();

    // 1. The runtime: one zip, unpacked into placement/pier/ that the engine
    //    looks for.
    let (url, bytes) = match RUNTIME.source {
        Source::Zip(url, bytes) => (url, bytes),
        Source::Files(_) => panic!("the runtime arrives as one archive"),
    };
    let runtime = root.join("piper");
    download::install_zip(url, bytes, &runtime, &cancel, &channel, phase::RUNTIME)
        .await
        .expect("the runtime downloads and unpacks");
    assert!(
        download::archive_ready(&runtime, RUNTIME.files, RUNTIME.dirs),
        "every runtime file and the phoneme folder are on disk"
    );
    let exe = runtime.join("piper.exe");
    assert!(exe.is_file(), "the binary is where the engine expects it");

    // 2. The voice: two files, sizes checked against the catalogue.
    let voice = catalog::find("en_US-amy-low").expect("a catalogued voice");
    let dir = root.join(voice.id);
    let spec = download::Spec {
        id: voice.id,
        files: voice.files(),
        bytes: voice.bytes,
        required: voice.required,
        engine: "piper",
    };
    download::install_spec(&spec, &dir, &cancel, &channel, phase::VOICE)
        .await
        .expect("the voice downloads");
    assert!(
        voice.required.iter().all(|name| dir.join(name).is_file()),
        "both voice files are on disk"
    );

    // 3. Speaking: a real child process, a real wave file, our own parser.
    let engine = SpeakEngine::default();
    let text = "Nova can read a reply aloud like this.";
    let synthesis = engine
        .synthesize(voice, &dir, &exe, text, None, 1.0)
        .await
        .expect("Piper speaks");
    assert_eq!(
        synthesis.rate, voice.sample_rate,
        "the catalogue's rate is the one measured"
    );
    assert!(
        !synthesis.samples.is_empty(),
        "there is audio in the wave file"
    );
    let seconds = synthesis.samples.len() as f32 / synthesis.rate as f32;
    assert!(
        seconds > 0.5,
        "a sentence is not {seconds} seconds of audio"
    );
    assert!(
        synthesis.samples.iter().any(|sample| sample.abs() > 0.01),
        "the samples are not silence"
    );
    eprintln!(
        "{} ms for {seconds:.2} s of {}-sample-rate audio",
        synthesis.ms, synthesis.rate
    );

    // 4. Progress reached the caller, as two phases. The guard is scoped to this
    // block rather than dropped by hand: clippy reads a binding that outlives it
    // as a lock held across the awaits below, and it is right to be suspicious.
    {
        let seen = frames.lock().expect("frames mutex");
        let phases: Vec<&str> = seen.iter().map(|frame| frame.phase).collect();
        assert!(
            phases.contains(&phase::RUNTIME),
            "the runtime phase was reported"
        );
        assert!(
            phases.contains(&phase::VOICE),
            "the voice phase was reported"
        );
        let last = seen.last().expect("at least one frame");
        assert_eq!(last.received, last.total, "the last frame is complete");
    }

    // 5. A voice with no files installed refuses instead of returning silence.
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).expect("an empty folder");
    let refusal = engine
        .synthesize(voice, &empty, &exe, text, None, 1.0)
        .await
        .expect_err("an uninstalled voice is refused");
    assert!(refusal.to_string().contains("not installed"), "{refusal}");

    let _ = std::fs::remove_dir_all(&root);
}
