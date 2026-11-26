//! The in-process ONNX voices, exercised for real: text in, samples out.
//!
//! Ignored because it needs kale-tts-1-nano's files on disk - about 20 MB from
//! Hugging Face, installed by Settings -> Voice, or pointed at directly with
//! NOVA_KALE_DIR. It exists because a green build says nothing about the things
//! that actually break here: whether onnx runtime can load a graph newer than
//! the one it was built for, whether the front end turns words into the ids the
//! graph expects, and whether anything audible comes back out.
//!
//! Run it with the voice installed, or with NOVA_KALE_DIR pointing at its
//! folder:
//!
//!   cargo test --test speak_onnx --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture

use std::path::PathBuf;

use novacode_lib::speak::frontend::Frontend;
use novacode_lib::speak::onnx;

/// The voice's folder: the environment's, so the test runs anywhere, and the
/// installed location as the default because that is where a real download puts
/// it.
fn voice_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOVA_KALE_DIR") {
        return PathBuf::from(dir);
    }
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata)
        .join("app.novacode.desktop")
        .join("speak")
        .join("kale-tts-1-nano")
}

fn installed() -> PathBuf {
    let dir = voice_dir();
    assert!(
        dir.join("kale-tts-1-nano.onnx").is_file(),
        "no kale files in {}: download the voice, or set NOVA_KALE_DIR",
        dir.display()
    );
    dir
}

const SENTENCE: &str = "The quick brown fox jumps over the lazy dog.";

#[test]
#[ignore = "needs kale-tts-1-nano's files on disk"]
fn kale_produces_audible_samples_at_its_own_rate() {
    let dir = installed();
    let synthesis = onnx::kale(&dir, SENTENCE, Some(0), 1.0).expect("kale should synthesise");

    assert_eq!(synthesis.rate, 24_000, "the rate is the graph's, not ours");
    // "The quick brown fox" is about three seconds; a floor of half a second
    // would pass on a single syllable, so this asks for more than that.
    assert!(
        synthesis.samples.len() > 24_000,
        "{} samples is under a second of audio",
        synthesis.samples.len()
    );
    let peak = synthesis
        .samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak > 0.01,
        "the loudest sample is {peak}, which is silence rather than speech"
    );
    let audible = synthesis
        .samples
        .iter()
        .filter(|sample| sample.abs() > 0.01)
        .count();
    assert!(
        audible * 10 > synthesis.samples.len(),
        "only {audible} of {} samples carry any sound",
        synthesis.samples.len()
    );
}

#[test]
#[ignore = "needs kale-tts-1-nano's files on disk"]
fn the_front_end_pronounces_a_word_the_list_has_not_got() {
    let dir = installed();
    let frontend = Frontend::load(&dir).expect("the front end should load");
    // A word nothing in the list can hold: the GRU has to spell it, and the
    // ids have to survive the id table, or the sentence would be dropped.
    let ids = frontend.ids("Kalecomputer synthesises novelwords");
    assert_eq!(ids.len(), 1, "one sentence, one graph input");
    assert!(
        ids[0].len() > 8,
        "{} ids is too few for three words",
        ids[0].len()
    );
    assert!(
        ids[0].iter().all(|id| (0..512).contains(id)),
        "an id outside the model's vocabulary"
    );
}

/// Supertonic's folder: the environment's, or the installed location.
fn supertonic_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOVA_SUPER_DIR") {
        return PathBuf::from(dir);
    }
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata)
        .join("app.novacode.desktop")
        .join("speak")
        .join("supertonic-3")
}

#[test]
#[ignore = "needs supertonic-3's files on disk (398 MB)"]
fn supertonic_speaks_through_four_graphs() {
    let dir = supertonic_dir();
    assert!(
        dir.join("vector_estimator.onnx").is_file(),
        "no supertonic files in {}: download the voice, or set NOVA_SUPER_DIR",
        dir.display()
    );
    let synthesis = novacode_lib::speak::supertonic::speak(
        &dir,
        "The quick brown fox jumps over the lazy dog.",
        None,
        1.0,
        "en",
    )
    .expect("supertonic should synthesise");

    assert_eq!(synthesis.rate, 44_100, "the rate comes from tts.json");
    assert!(
        synthesis.samples.len() > 22_050,
        "{} samples is under half a second",
        synthesis.samples.len()
    );
    let peak = synthesis
        .samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak > 0.01,
        "the loudest sample is {peak}, which is silence rather than speech"
    );
}

#[test]
#[ignore = "needs supertonic-3's files on disk (398 MB)"]
fn supertonic_speaks_another_language() {
    let dir = supertonic_dir();
    // The models carry thirty-one languages and the tag is the whole switch, so
    // a non-English sentence is the test that the plumbing is not hard-wired to
    // English. The card itself offers English only.
    let synthesis = novacode_lib::speak::supertonic::speak(
        &dir,
        "Buenos dias, esto es una prueba de la voz en espanol.",
        None,
        1.0,
        "es",
    )
    .expect("supertonic should speak Spanish");
    assert_eq!(synthesis.rate, 44_100);
    let peak = synthesis
        .samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak > 0.01,
        "the loudest sample is {peak}, which is silence rather than speech"
    );
}

#[test]
#[ignore = "needs supertonic-3 on disk plus a second style (M2.json)"]
fn two_styles_are_two_voices() {
    let dir = supertonic_dir();
    if !dir.join("M2.json").is_file() {
        // Not a silent skip: the assertion below is the point, so say what is
        // missing instead of passing on nothing.
        panic!(
            "{} has no M2.json: download a second style to run this test",
            dir.display()
        );
    }
    let first =
        novacode_lib::speak::supertonic::speak(&dir, "Reading this twice.", Some(0), 1.0, "en")
            .expect("style 0");
    let second =
        novacode_lib::speak::supertonic::speak(&dir, "Reading this twice.", Some(1), 1.0, "en")
            .expect("style 1");
    assert_eq!(first.rate, second.rate);
    assert!(
        first.samples != second.samples,
        "two styles produced identical audio, so the style file is not being used"
    );
}

/// Kokoro's folder: the environment's, or the installed location.
fn kokoro_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOVA_KOKORO_DIR") {
        return PathBuf::from(dir);
    }
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata)
        .join("app.novacode.desktop")
        .join("speak")
        .join("kokoro-82m")
}

#[test]
#[ignore = "needs Kokoro's graph and the shared front end beside it"]
fn kokoro_speaks_through_the_shared_front_end() {
    let dir = kokoro_dir();
    assert!(
        dir.join("model_quantized.onnx").is_file(),
        "no Kokoro files in {}: download the voice, or set NOVA_KOKORO_DIR",
        dir.display()
    );
    // The front end is found at <voice>/../frontend, so this test also proves
    // the shared layout rather than a same-folder shortcut.
    let frontend = dir
        .parent()
        .map(|parent| parent.join("frontend"))
        .unwrap_or_default();
    assert!(
        frontend.join("lexicon.tsv").is_file(),
        "the shared front end is not at {}",
        frontend.display()
    );

    let synthesis = novacode_lib::speak::kokoro::speak(
        &dir,
        "The quick brown fox jumps over the lazy dog.",
        1.0,
    )
    .expect("Kokoro should synthesise");

    assert_eq!(synthesis.rate, 24_000, "Kokoro's own rate");
    assert!(
        synthesis.samples.len() > 12_000,
        "{} samples is under half a second",
        synthesis.samples.len()
    );
    let peak = synthesis
        .samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak > 0.01,
        "the loudest sample is {peak}, which is silence rather than speech"
    );
}

#[test]
#[ignore = "needs kale-tts-1-nano's files on disk"]
fn a_preset_that_was_not_downloaded_is_refused_rather_than_substituted() {
    let dir = installed();
    // Only one preset's style table is downloaded, so a second speaker has no
    // audio to speak with. Reusing the first preset silently would be a voice
    // pretending to be another one.
    let error = onnx::kale(&dir, "Reading this aloud twice.", Some(1), 1.0)
        .expect_err("speaker 1 has no style table here");
    let message = error.to_string();
    assert!(
        message.contains(".bin"),
        "the refusal should name the missing style table, and said: {message}"
    );
}
