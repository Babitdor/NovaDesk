//! The speech-service path, against a real service.
//!
//! Ignored because it makes one HTTPS request. The request carries a junk key and
//! one word of text: what it proves is that the status code is read at all, that a
//! rejected key becomes a sentence instead of an empty reply, and that the key
//! never had to appear on a command line to get there.

use novacode_lib::speak::service::{self, Profile, Provider};

#[tokio::test]
#[ignore = "one real POST to api.openai.com"]
async fn a_rejected_key_is_a_sentence() {
    let profile = Profile {
        provider: Provider::OpenAi,
        model: "gpt-4o-mini-tts".to_string(),
        voice: "alloy".to_string(),
        url: String::new(),
        key: "sk-nova-is-not-a-real-key".to_string(),
        instructions: String::new(),
    };
    let error = service::synthesize(&profile, "test", 1.0)
        .await
        .expect_err("a junk key is refused");
    let message = error.to_string();
    assert!(message.contains("rejected the key"), "{message}");
}

#[tokio::test]
#[ignore = "one real POST to an https address that is not a speech service"]
async fn a_reply_that_is_not_audio_is_refused() {
    // example.com answers 200 with HTML for any path, which is exactly the shape
    // of failure a misconfigured custom server produces.
    let profile = Profile {
        provider: Provider::Custom,
        model: "any".to_string(),
        voice: "any".to_string(),
        url: "https://example.com/v1/audio/speech".to_string(),
        key: "not-needed".to_string(),
        instructions: String::new(),
    };
    let error = service::synthesize(&profile, "test", 1.0)
        .await
        .expect_err("HTML is not audio");
    let message = error.to_string();
    assert!(
        message.contains("not audio") || message.contains("said"),
        "{message}"
    );
}

#[test]
fn a_profile_without_a_key_never_reaches_the_network() {
    let profile = Profile {
        provider: Provider::OpenAi,
        model: "gpt-4o-mini-tts".to_string(),
        voice: "alloy".to_string(),
        url: String::new(),
        key: String::new(),
        instructions: String::new(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let error = runtime
        .block_on(service::synthesize(&profile, "test", 1.0))
        .expect_err("no key, no request");
    assert!(error.to_string().contains("no API key"), "{error}");
}
