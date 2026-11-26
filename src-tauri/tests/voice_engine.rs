//! The voice capture path, end to end.
//!
//! The resampling test is always run: it is the piece that turns whatever the
//! microphone produced into the 16 kHz mono every engine expects. The inference
//! test is ignored because it needs a downloaded model and a recording, and it
//! exists to prove ONNX Runtime actually links and run - a green build alone
//! does not show that.

use std::path::PathBuf;

use novacode_lib::voice::audio::{downmix, to_model_input, TARGET_RATE};

const RATE: u32 = 48_000;

fn stereo_ramp(frames: usize) -> Vec<f32> {
    let mut interleaved = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let value = frame as f32 / frames as f32;
        interleaved.push(value);
        interleaved.push(value);
    }
    interleaved
}

#[test]
fn resamples_a_48k_stereo_ramp_to_16k_mono() {
    let frames = RATE as usize;
    let interleaved = stereo_ramp(frames);

    let mono = downmix(&interleaved, 2);
    assert_eq!(mono.len(), frames, "downmix keeps one sample per frame");
    assert!((mono[100] - 100.0 / frames as f32).abs() < 1e-6);

    let output = to_model_input(&interleaved, 2, RATE).expect("a 48 kHz stereo ramp is valid");
    let expected_len = frames * TARGET_RATE as usize / RATE as usize;
    assert_eq!(output.len(), expected_len, "one third of the input frames");
    let ratio = output.len() as f32 / frames as f32;
    assert!((ratio - 1.0 / 3.0).abs() < 1e-4, "length ratio is {ratio}");

    // Each output sample averages the three input samples it covers, so a ramp
    // keeps its amplitude and its slope instead of collapsing towards zero.
    for (index, value) in output.iter().enumerate() {
        let expected = (index as f32 * 3.0 + 1.0) / frames as f32;
        assert!(
            (value - expected).abs() < 1e-3,
            "sample {index}: {value} should be {expected}"
        );
    }
}

#[test]
fn rejects_audio_longer_than_the_limit() {
    let frames = (novacode_lib::voice::audio::MAX_SECONDS as usize + 1) * 16_000;
    let samples = vec![0.0_f32; frames];
    let error = to_model_input(&samples, 1, TARGET_RATE).expect_err("an hour is refused");
    assert!(error.to_string().contains("limit"), "{error}");
}

/// Runs a real model over a real recording:
///
/// NOVA_VOICE_MODEL=<dir with the extracted model files>
/// NOVA_VOICE_WAV=<16 kHz mono wav>
/// NOVA_VOICE_EXPECT=<a word the recording says, default "the">
/// cargo test --test voice_engine -- --ignored --nocapture
#[test]
#[ignore = "needs NOVA_VOICE_MODEL and NOVA_VOICE_WAV"]
fn transcribes_a_real_recording() {
    let dir = PathBuf::from(std::env::var("NOVA_VOICE_MODEL").expect("NOVA_VOICE_MODEL"));
    let wav = PathBuf::from(std::env::var("NOVA_VOICE_WAV").expect("NOVA_VOICE_WAV"));
    let model_id =
        std::env::var("NOVA_VOICE_MODEL_ID").unwrap_or_else(|_| "canary-180m-flash".to_string());
    let expected = std::env::var("NOVA_VOICE_EXPECT").unwrap_or_else(|_| "the".to_string());

    let model = novacode_lib::voice::catalog::find(&model_id).expect("a catalogued model");
    let samples = transcribe_rs::audio::read_wav_samples(&wav).expect("a readable wav");
    assert!(!samples.is_empty(), "the wav carried no samples");

    let engine = novacode_lib::voice::engine::VoiceEngine::default();
    let result = engine
        .transcribe(model, &dir, None, &samples)
        .expect("the model transcribes the recording");

    println!("{} said: {:?} in {} ms", model.name, result.text, result.ms);
    assert!(!result.text.trim().is_empty(), "the transcript was empty");
    assert!(
        result
            .text
            .to_lowercase()
            .contains(&expected.to_lowercase()),
        "expected {expected:?} in {:?}",
        result.text
    );
}
