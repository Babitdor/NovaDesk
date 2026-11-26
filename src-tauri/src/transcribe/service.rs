//! Speech-to-text through a hosted service, instead of a model on this computer.
//!
//! Four things shape this module:
//!
//! - OpenAI, Groq and Mistral publish the same request shape for transcription:
//!   a multipart form with the recording in the field named 'file', the model
//!   in 'model' and the language, optional, in 'language'. The table below is
//!   the only place those addresses and model ids are written down, taken from
//!   each provider's API reference (platform.openai.com audio reference,
//!   console.groq.com/docs/speech-to-text, docs.mistral.ai audio transcriptions).
//! - The form is described by a pure function, so what is sent is a test rather
//!   than a hope. The custom provider is expected to speak the same shape.
//! - The key never reaches argv: it becomes a header file that is deleted when
//!   the transfer ends (see crate::http).
//! - A 200 is not a transcription. Every one of these services answers a bad
//!   request with JSON and a 200 sometimes, so the answer is read for its text
//!   field and a body without one is a failure with the service's own sentence.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;
use crate::http;
use crate::voice::engine::Transcription;

/// A transcript is short; a megabyte of JSON is not one.
const LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAi,
    Groq,
    Mistral,
    Custom,
}

/// What the settings page needs to draw a provider's row.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: Provider,
    pub name: &'static str,
    pub url: &'static str,
    pub models: &'static [&'static str],
    pub note: &'static str,
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: Provider::OpenAi,
        name: "OpenAI",
        url: "https://api.openai.com/v1/audio/transcriptions",
        models: &["gpt-4o-transcribe", "gpt-4o-mini-transcribe", "whisper-1"],
        note: "The speech-to-text models on the same account as the chat models.",
    },
    ProviderInfo {
        id: Provider::Groq,
        name: "Groq",
        url: "https://api.groq.com/openai/v1/audio/transcriptions",
        models: &["whisper-large-v3", "whisper-large-v3-turbo"],
        note: "Whisper on Groq: the same endpoint shape, answered faster.",
    },
    ProviderInfo {
        id: Provider::Mistral,
        name: "Mistral",
        url: "https://api.mistral.ai/v1/audio/transcriptions",
        models: &["voxtral-mini-latest", "voxtral-mini-2507"],
        note: "Voxtral: Mistral's own transcription model, no format field.",
    },
    ProviderInfo {
        id: Provider::Custom,
        name: "Your own server",
        url: "",
        models: &[],
        note: "Any server that speaks OpenAI's /v1/audio/transcriptions shape, on https.",
    },
];

/// One saved transcription service, as the page stores it. The key comes over
/// with it: it lives in the webview's storage, which is where the model API
/// keys already live.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub provider: Provider,
    #[serde(default)]
    pub model: String,
    /// Only read for the custom provider; the other three are published.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub key: String,
}

/// Where the request goes. A custom server is whatever the user saved.
pub fn endpoint(profile: &Profile) -> Result<String, AppError> {
    match profile.provider {
        Provider::OpenAi => Ok("https://api.openai.com/v1/audio/transcriptions".to_string()),
        Provider::Groq => Ok("https://api.groq.com/openai/v1/audio/transcriptions".to_string()),
        Provider::Mistral => Ok("https://api.mistral.ai/v1/audio/transcriptions".to_string()),
        Provider::Custom => {
            let url = profile.url.trim();
            if url.is_empty() {
                return Err(AppError::Voice(
                    "the profile has no address to send to".to_string(),
                ));
            }
            Ok(url.to_string())
        }
    }
}

/// A language code a hosted service accepts: two or three letters, ISO-639-1 or
/// its three-letter form. The dictation language filter also offers entries such
/// as "all", which is not a language and is simply not sent.
pub fn language_field(code: Option<&str>) -> Option<&str> {
    let code = code?.trim();
    // The dictation filter's "every language" entry: three letters, and not one
    // of them a language.
    if code.eq_ignore_ascii_case("all") {
        return None;
    }
    if (2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphabetic()) {
        Some(code)
    } else {
        None
    }
}

/// The text fields of the form. The recording is the fourth field and is
/// uploaded as a file, which is why it is not here.
///
/// Every published provider takes this shape; the provider is still a parameter
/// so a future one that does not say so cannot be added quietly.
pub fn form_fields<'a>(
    provider: Provider,
    model: &'a str,
    language: Option<&'a str>,
) -> Vec<(&'static str, &'a str)> {
    let mut fields = vec![("model", model)];
    match provider {
        // Same fields on purpose: three services, one shape.
        Provider::OpenAi | Provider::Groq | Provider::Mistral | Provider::Custom => {
            if let Some(code) = language_field(language) {
                fields.push(("language", code));
            }
        }
    }
    fields
}

/// A 16-bit mono WAV of what the microphone produced, at the rate the engine
/// layer already normalised to.
///
/// The same bytes the page builds for its own test playback (see toWav in
/// src/lib/voice.ts), so a recording sent to a service and one played back here
/// are the same audio.
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data);
    let push_u32 = |bytes: &mut Vec<u8>, value: u32| bytes.extend_from_slice(&value.to_le_bytes());
    let push_u16 = |bytes: &mut Vec<u8>, value: u16| bytes.extend_from_slice(&value.to_le_bytes());
    bytes.extend_from_slice(b"RIFF");
    push_u32(&mut bytes, 36 + data as u32);
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    push_u32(&mut bytes, 16);
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 1);
    push_u32(&mut bytes, rate);
    push_u32(&mut bytes, rate * 2);
    push_u16(&mut bytes, 2);
    push_u16(&mut bytes, 16);
    bytes.extend_from_slice(b"data");
    push_u32(&mut bytes, data as u32);
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let scaled = if clamped < 0.0 {
            clamped * 0x8000 as f32
        } else {
            clamped * 0x7fff as f32
        };
        push_u16(&mut bytes, scaled as i16 as u16);
    }
    bytes
}

fn describe(status: u16) -> String {
    match status {
        401 | 403 => {
            "the transcription service rejected the key; check it in Settings, Voice".to_string()
        }
        404 => "the transcription service does not know that model".to_string(),
        413 | 422 => "the transcription service refused the recording".to_string(),
        429 => "the transcription service is rate limited; try again shortly".to_string(),
        500..=599 => format!("the transcription service failed ({status})"),
        other => format!("the transcription service said {other}"),
    }
}

/// The transcript out of an answer, or the reason there is none.
pub fn read_answer(status: u16, body: &[u8]) -> Result<String, AppError> {
    if !(200..=299).contains(&status) {
        // These services put the useful sentence in the body; a rejected key
        // is more often a body than a status, so use it when it is there.
        return Err(AppError::Voice(match complaint(body) {
            Some(sentence) => format!("{} ({sentence})", describe(status)),
            None => describe(status),
        }));
    }
    let parsed: Value = serde_json::from_slice(body).map_err(|_| {
        AppError::Voice(
            "the transcription service answered with something that is not a transcript"
                .to_string(),
        )
    })?;
    if let Some(text) = parsed.get("text").and_then(Value::as_str) {
        return Ok(text.trim().to_string());
    }
    if let Some(sentence) = parsed
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return Err(AppError::Voice(sentence.to_string()));
    }
    Err(AppError::Voice(
        "the transcription service answered without a transcript".to_string(),
    ))
}

fn complaint(body: &[u8]) -> Option<String> {
    let parsed: Value = serde_json::from_slice(body).ok()?;
    parsed
        .get("error")?
        .get("message")?
        .as_str()
        .map(str::to_string)
}

/// Everything that can be refused before a byte leaves the machine. Pure, so
/// each refusal is a test with no network and no runtime in the way.
pub fn check(profile: &Profile, samples: &[f32], rate: u32) -> Result<(), AppError> {
    if samples.is_empty() {
        return Err(AppError::Voice("the recording is empty".to_string()));
    }
    if rate == 0 {
        return Err(AppError::Voice(
            "the recording reported no sample rate".to_string(),
        ));
    }
    // A server the user runs may take no model and no key; the published
    // services need both, and refusing here keeps the recording on disk.
    if profile.provider != Provider::Custom {
        if profile.model.trim().is_empty() {
            return Err(AppError::Voice(
                "this transcription profile has no model yet".to_string(),
            ));
        }
        if profile.key.trim().is_empty() {
            return Err(AppError::Voice(
                "this transcription profile has no API key yet".to_string(),
            ));
        }
    }
    Ok(())
}

/// Transcribe one recording through a saved service.
pub async fn transcribe(
    profile: &Profile,
    samples: &[f32],
    rate: u32,
    language: Option<&str>,
) -> Result<Transcription, AppError> {
    check(profile, samples, rate)?;
    let url = endpoint(profile)?;
    let key = profile.key.trim();
    // A keyless server sends no Authorization header at all.
    let header = if key.is_empty() {
        None
    } else {
        Some(format!("Authorization: Bearer {key}"))
    };
    let fields = form_fields(profile.provider, profile.model.trim(), language);
    // 300 s of 16 kHz mono PCM is 9.6 MB, so the duration limit in
    // voice::audio is what bounds this upload; nothing is read back as audio.
    let audio = wav(samples, rate);

    let start = Instant::now();
    let (bytes, status) = http::post_form_for_bytes(
        &url,
        header.as_deref(),
        ("file", &audio, "audio/wav"),
        &fields,
        LIMIT,
    )
    .await?;
    let ms = start.elapsed().as_millis() as u64;
    let text = read_answer(status, &bytes)?;
    if text.is_empty() {
        return Err(AppError::Voice(
            "nothing was recognised in that recording".to_string(),
        ));
    }
    Ok(Transcription { text, ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(provider: Provider) -> Profile {
        Profile {
            provider,
            model: "m".to_string(),
            url: "https://speech.example.com/v1/audio/transcriptions".to_string(),
            key: "k".to_string(),
        }
    }

    #[test]
    fn the_published_addresses_are_the_ones_in_the_provider_table() {
        for info in PROVIDERS.iter().filter(|info| info.id != Provider::Custom) {
            assert_eq!(endpoint(&profile(info.id)).expect("an endpoint"), info.url);
        }
    }

    #[test]
    fn a_custom_profile_needs_an_address() {
        let mut empty = profile(Provider::Custom);
        empty.url = String::new();
        assert!(endpoint(&empty).is_err());
        assert_eq!(
            endpoint(&profile(Provider::Custom)).expect("a url"),
            "https://speech.example.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn every_provider_is_sent_the_same_form() {
        for info in PROVIDERS.iter() {
            let fields = form_fields(info.id, "model-x", Some("en"));
            assert_eq!(fields[0], ("model", "model-x"), "{:?}", info.id);
            assert_eq!(fields[1], ("language", "en"), "{:?}", info.id);
            assert!(
                fields.iter().all(|(name, _)| *name != "file"),
                "the recording is uploaded, not typed into a field"
            );
        }
    }

    #[test]
    fn only_a_language_code_is_sent_as_one() {
        let fields = |language| form_fields(Provider::OpenAi, "m", language);
        assert_eq!(fields(Some("de")).len(), 2);
        assert_eq!(fields(Some("yue")).len(), 2);
        assert_eq!(fields(Some("all")).len(), 1, "not a language");
        assert_eq!(fields(Some("english")).len(), 1);
        assert_eq!(fields(Some("")).len(), 1);
        assert_eq!(fields(Some("e1")).len(), 1);
        assert_eq!(fields(None).len(), 1);
    }

    #[test]
    fn the_wav_header_is_the_one_a_player_expects() {
        let bytes = wav(&[0.0, 0.5, -0.5], 16_000);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            36 + 6
        );
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1, "mono");
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            16_000
        );
        assert_eq!(
            u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
            32_000
        );
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            6
        );
        assert_eq!(bytes.len(), 50);
        assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 16_383);
        assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), -16_384);
    }

    #[test]
    fn a_sample_beyond_full_scale_is_clipped_not_wrapped() {
        let bytes = wav(&[4.0, -4.0], 16_000);
        assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 0x7fff);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), -0x8000);
    }

    #[test]
    fn the_transcript_is_the_text_field() {
        let text = read_answer(200, br#"{"text":"  hello there  "}"#).expect("a transcript");
        assert_eq!(text, "hello there");
    }

    #[test]
    fn a_body_without_a_transcript_is_not_a_success() {
        assert!(read_answer(200, br#"{"error":"nope"}"#).is_err());
        assert!(read_answer(200, b"not json").is_err());
        assert_eq!(
            read_answer(200, br#"{"text":"   "}"#).expect("a transcript"),
            "",
            "whitespace is not text; transcribe is what refuses it"
        );
    }

    #[test]
    fn an_error_keeps_the_services_own_sentence() {
        let err = read_answer(
            401,
            br#"{"error":{"message":"Incorrect API key provided"}}"#,
        )
        .expect_err("a rejected key");
        let sentence = err.to_string();
        assert!(sentence.contains("rejected the key"), "{sentence}");
        assert!(
            sentence.contains("Incorrect API key provided"),
            "{sentence}"
        );
        let quiet = read_answer(429, b"").expect_err("rate limited").to_string();
        assert!(quiet.contains("rate limited"), "{quiet}");
    }

    #[test]
    fn nothing_is_sent_for_an_empty_recording() {
        let err = check(&profile(Provider::OpenAi), &[], 16_000).expect_err("empty");
        assert!(err.to_string().contains("empty"), "{err}");
        let rate = check(&profile(Provider::OpenAi), &[0.1], 0).expect_err("no rate");
        assert!(rate.to_string().contains("sample rate"), "{rate}");
    }

    #[test]
    fn a_profile_without_a_key_or_a_model_is_refused_before_any_transfer() {
        let mut keyless = profile(Provider::OpenAi);
        keyless.key = String::new();
        let err = check(&keyless, &[0.1], 16_000).expect_err("no key");
        assert!(err.to_string().contains("no API key"), "{err}");

        let mut modelless = profile(Provider::Mistral);
        modelless.model = String::new();
        let err = check(&modelless, &[0.1], 16_000).expect_err("no model");
        assert!(err.to_string().contains("no model"), "{err}");

        // A custom server may want no model and no key; endpoint() is where
        // its address is checked.
        let mut custom = profile(Provider::Custom);
        custom.model = String::new();
        custom.key = String::new();
        assert!(check(&custom, &[0.1], 16_000).is_ok());
    }
}
