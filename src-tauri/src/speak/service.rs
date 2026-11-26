//! Speech services: reading a reply aloud through a hosted text-to-speech API
//! instead of a voice on this computer.
//!
//! Three things shape this module:
//!
//! - The body is built by a pure function, so each provider's shape is a test
//!   rather than a hope. OpenAI, Groq and an OpenAI-compatible server all take
//!   the same JSON; ElevenLabs takes a different one and puts the voice in the
//!   path.
//! - The key never reaches argv. It becomes a header file, written 0600 and
//!   deleted when the transfer ends (see `crate::http`).
//! - A 200 that is not audio is a failure. Every one of these services answers
//!   an error with JSON and a 200 sometimes; playing that JSON is impossible,
//!   and claiming success while the page stays silent is worse than saying so.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::AppError;
use crate::http;

/// A four-minute reply at 128 kbit/s is about 4 MB; anything larger is not a
/// reply being read aloud.
const LIMIT: u64 = 16 * 1024 * 1024;
/// The same ceiling the on-device path uses: one request, one paragraph.
pub const MAX_CHARS: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAi,
    Groq,
    ElevenLabs,
    Custom,
}

/// What the settings page needs to draw a provider's row: where it points, which
/// models and voices it offers, and whether it takes a style instruction.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: Provider,
    pub name: &'static str,
    pub url: &'static str,
    pub models: &'static [&'static str],
    pub voices: &'static [&'static str],
    /// Whether `instructions` reaches the service; only OpenAI's speech API has
    /// a field for it.
    pub instructions: bool,
    /// The audio the service returns, as the page's blob type.
    pub format: &'static str,
    pub note: &'static str,
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: Provider::OpenAi,
        name: "OpenAI",
        url: "https://api.openai.com/v1/audio/speech",
        models: &["gpt-4o-mini-tts", "tts-1", "tts-1-hd"],
        voices: &[
            "alloy", "ash", "ballad", "coral", "echo", "fable", "onyx", "nova", "sage", "shimmer",
        ],
        instructions: true,
        format: "wav",
        note: "Takes a style instruction, so \"read it as a tired engineer\" works.",
    },
    ProviderInfo {
        id: Provider::Groq,
        name: "Groq",
        url: "https://api.groq.com/openai/v1/audio/speech",
        models: &["playai-tts", "playai-tts-arabic"],
        voices: &[
            "Arista-PlayAI",
            "Atlas-PlayAI",
            "Basil-PlayAI",
            "Briggs-PlayAI",
            "Calum-PlayAI",
            "Celeste-PlayAI",
            "Cheyenne-PlayAI",
            "Chip-PlayAI",
            "Cillian-PlayAI",
            "Deedee-PlayAI",
            "Fritz-PlayAI",
            "Gail-PlayAI",
            "Indigo-PlayAI",
            "Mamaw-PlayAI",
            "Mason-PlayAI",
            "Mikail-PlayAI",
            "Mitch-PlayAI",
            "Nia-PlayAI",
            "Quinn-PlayAI",
            "Ruby-PlayAI",
            "Thunder-PlayAI",
        ],
        instructions: false,
        format: "wav",
        note: "The same request shape as OpenAI, on Groq's faster endpoint.",
    },
    ProviderInfo {
        id: Provider::ElevenLabs,
        name: "ElevenLabs",
        url: "https://api.elevenlabs.io/v1/text-to-speech",
        models: &[
            "eleven_multilingual_v2",
            "eleven_turbo_v2_5",
            "eleven_flash_v2_5",
        ],
        voices: &[],
        instructions: false,
        format: "mp3",
        note: "Paste a voice id from your ElevenLabs account; it goes in the URL.",
    },
    ProviderInfo {
        id: Provider::Custom,
        name: "Your own server",
        url: "",
        models: &[],
        voices: &[],
        instructions: false,
        format: "wav",
        note: "Any server that speaks OpenAI's /v1/audio/speech shape, on https.",
    },
];

/// One saved speech service, as the page stores it. The key comes over with it:
/// it lives in the webview's storage, which is the same place the model API keys
/// already live.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub provider: Provider,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub voice: String,
    /// Only read for `Custom`; the other three have published endpoints.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub instructions: String,
}

#[derive(Debug, Clone)]
pub struct Audio {
    pub bytes: Vec<u8>,
    /// "mp3" or "wav", read from the bytes rather than trusted from the profile.
    pub format: &'static str,
    pub ms: u64,
}

/// Where the request goes. ElevenLabs puts the voice in the path, and a custom
/// server is whatever the user saved.
pub fn endpoint(profile: &Profile) -> Result<String, AppError> {
    match profile.provider {
        Provider::OpenAi => Ok("https://api.openai.com/v1/audio/speech".to_string()),
        Provider::Groq => Ok("https://api.groq.com/openai/v1/audio/speech".to_string()),
        Provider::ElevenLabs => {
            let voice = profile.voice.trim();
            if voice.is_empty() {
                return Err(AppError::Speak(
                    "the ElevenLabs profile needs a voice id".to_string(),
                ));
            }
            // An id is letters, digits, - and _; anything else would rewrite the
            // path or the query of the address the key ends up going to.
            if !voice
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(AppError::Speak(
                    "the ElevenLabs voice id can only hold letters, digits, - and _".to_string(),
                ));
            }
            Ok(format!(
                "https://api.elevenlabs.io/v1/text-to-speech/{voice}?output_format=mp3_44100_128"
            ))
        }
        Provider::Custom => {
            let url = profile.url.trim();
            if url.is_empty() {
                return Err(AppError::Speak(
                    "the profile has no address to send to".to_string(),
                ));
            }
            Ok(url.to_string())
        }
    }
}

/// The JSON body for one request, which is the whole of a provider's contract.
///
/// OpenAI, Groq and a compatible server share the speech shape; ElevenLabs takes
/// a different one and no speed, because its model has none.
pub fn request_body(
    provider: Provider,
    model: &str,
    voice: &str,
    text: &str,
    speed: f32,
    instructions: &str,
) -> Value {
    let speed = speed.clamp(0.5, 2.0);
    match provider {
        Provider::OpenAi | Provider::Groq | Provider::Custom => {
            let mut body = json!({
                "model": model,
                "input": text,
                "voice": voice,
                "response_format": "wav",
                "speed": speed,
            });
            // Only the model that has a field for it gets the instruction; the
            // others reject the request when an unknown key is present.
            if provider == Provider::OpenAi && !instructions.trim().is_empty() {
                body["instructions"] = json!(instructions.trim());
            }
            body
        }
        Provider::ElevenLabs => json!({
            "text": text,
            "model_id": model,
            "voice_settings": { "stability": 0.5, "similarity_boost": 0.75 },
        }),
    }
}

/// What is allowed to be sent at all. Pure, so both destinations and the tests
/// agree on it without a network in the way.
pub fn check_text(text: &str) -> Result<&str, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Speak(
            "there was nothing to read aloud".to_string(),
        ));
    }
    if text.chars().count() > MAX_CHARS {
        return Err(AppError::Speak(format!(
            "that reply is longer than the {} characters one request reads",
            MAX_CHARS
        )));
    }
    Ok(text)
}

/// Read one piece of text aloud through a saved service.
pub async fn synthesize(profile: &Profile, text: &str, speed: f32) -> Result<Audio, AppError> {
    let text = check_text(text)?;
    let key = profile.key.trim();
    // A server the user runs may take no key; the published ones do.
    if key.is_empty() && profile.provider != Provider::Custom {
        return Err(AppError::Speak(
            "this speech profile has no API key yet".to_string(),
        ));
    }

    let url = endpoint(profile)?;
    http::https_url(&url)?;
    let body = request_body(
        profile.provider,
        profile.model.trim(),
        profile.voice.trim(),
        text,
        speed,
        &profile.instructions,
    );
    let payload = serde_json::to_string(&body)
        .map_err(|err| AppError::Speak(format!("the request could not be described: {err}")))?;
    // ElevenLabs names its key header differently from everyone else; both
    // travel in a file, never in argv. A keyless server sends none.
    let header = if key.is_empty() {
        None
    } else {
        Some(match profile.provider {
            Provider::ElevenLabs => format!("xi-api-key: {key}"),
            _ => format!("Authorization: Bearer {key}"),
        })
    };

    let start = Instant::now();
    let (bytes, status) = http::post_for_bytes(&url, header.as_deref(), &payload, LIMIT).await?;
    let ms = start.elapsed().as_millis() as u64;

    if !(200..=299).contains(&status) {
        return Err(AppError::Speak(describe(status)));
    }
    let format = audio_format(&bytes).ok_or_else(|| {
        AppError::Speak(
            "the speech service answered with something that is not audio; check the model and voice"
                .to_string(),
        )
    })?;
    Ok(Audio { bytes, format, ms })
}

fn describe(status: u16) -> String {
    match status {
        401 | 403 => "the speech service rejected the key; check it in Settings, Voice".to_string(),
        404 => "the speech service does not know that model or voice".to_string(),
        413 => "that reply is longer than the speech service accepts".to_string(),
        429 => "the speech service is rate limited; try again shortly".to_string(),
        500..=599 => format!("the speech service failed ({status})"),
        other => format!("the speech service said {other}"),
    }
}

/// The container, from its first bytes. The magic is the honest answer: a
/// mislabelled profile still produces audio the page can play.
pub fn audio_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"RIFF") {
        return Some("wav");
    }
    if bytes.starts_with(b"ID3") {
        return Some("mp3");
    }
    if bytes.len() > 1 && bytes[0] == 0xff && (bytes[1] & 0xe0) == 0xe0 {
        return Some("mp3");
    }
    if bytes.starts_with(b"OggS") {
        return Some("ogg");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(provider: Provider) -> Profile {
        Profile {
            provider,
            model: "m".to_string(),
            voice: "v".to_string(),
            url: "https://speech.example.com/v1/audio/speech".to_string(),
            key: "k".to_string(),
            instructions: String::new(),
        }
    }

    #[test]
    fn openai_sends_the_speech_shape_and_its_instruction() {
        let body = request_body(
            Provider::OpenAi,
            "gpt-4o-mini-tts",
            "alloy",
            "Hello",
            1.25,
            "  dry  ",
        );
        assert_eq!(body["model"], "gpt-4o-mini-tts");
        assert_eq!(body["input"], "Hello");
        assert_eq!(body["voice"], "alloy");
        assert_eq!(body["speed"], 1.25);
        assert_eq!(body["instructions"], "dry");
    }

    #[test]
    fn groq_gets_no_instruction_field() {
        let body = request_body(
            Provider::Groq,
            "playai-tts",
            "Fritz-PlayAI",
            "Hello",
            1.0,
            "dry",
        );
        assert!(
            body.get("instructions").is_none(),
            "Groq rejects unknown keys"
        );
        assert_eq!(body["response_format"], "wav");
    }

    #[test]
    fn elevenlabs_sends_its_own_shape_and_no_speed() {
        let body = request_body(
            Provider::ElevenLabs,
            "eleven_multilingual_v2",
            "abc",
            "Hello",
            2.0,
            "",
        );
        assert_eq!(body["text"], "Hello");
        assert_eq!(body["model_id"], "eleven_multilingual_v2");
        assert!(body.get("input").is_none());
        assert!(
            body.get("speed").is_none(),
            "the model has no speed control"
        );
        assert_eq!(body["voice_settings"]["stability"], 0.5);
    }

    #[test]
    fn a_custom_server_gets_the_openai_shape_without_the_instruction() {
        let body = request_body(Provider::Custom, "kokoro", "af_sky", "Hello", 0.5, "dry");
        assert_eq!(body["model"], "kokoro");
        assert_eq!(body["voice"], "af_sky");
        assert!(body.get("instructions").is_none());
    }

    #[test]
    fn speed_is_clamped_for_every_one_of_them() {
        assert_eq!(
            request_body(Provider::OpenAi, "m", "v", "x", 9.0, ""),
            request_body(Provider::OpenAi, "m", "v", "x", 2.0, "")
        );
        assert_eq!(
            request_body(Provider::OpenAi, "m", "v", "x", 0.1, ""),
            request_body(Provider::OpenAi, "m", "v", "x", 0.5, "")
        );
    }

    #[test]
    fn elevenlabs_puts_the_voice_in_the_path() {
        let url = endpoint(&profile(Provider::ElevenLabs)).expect("a voice id is set");
        assert!(url.starts_with("https://api.elevenlabs.io/v1/text-to-speech/v?"));
        let mut missing = profile(Provider::ElevenLabs);
        missing.voice = String::new();
        assert!(endpoint(&missing).is_err());
        // A path or a query in the id would move the request; refuse it.
        let mut escaped = profile(Provider::ElevenLabs);
        escaped.voice = "abc/../../other?x=1".to_string();
        let err = endpoint(&escaped).expect_err("an id that is not an id");
        assert!(err.to_string().contains("voice id"), "{err}");
    }

    #[test]
    fn a_custom_profile_needs_an_address() {
        let mut empty = profile(Provider::Custom);
        empty.url = String::new();
        assert!(endpoint(&empty).is_err());
        assert_eq!(
            endpoint(&profile(Provider::Custom)).expect("a url"),
            "https://speech.example.com/v1/audio/speech"
        );
    }

    #[test]
    fn the_published_endpoints_are_the_ones_in_the_provider_table() {
        for info in PROVIDERS.iter().filter(|info| info.id != Provider::Custom) {
            let url = endpoint(&profile(info.id)).expect("an endpoint");
            // ElevenLabs appends the voice id and a format to the address the
            // table publishes, so the table's address is a prefix there; the
            // other two are the address itself.
            assert!(
                url == info.url || url.starts_with(&format!("{}/", info.url)),
                "{:?} sends to {url}, which is not {}",
                info.id,
                info.url
            );
        }
    }

    #[test]
    fn a_body_that_is_not_audio_is_not_a_success() {
        assert_eq!(audio_format(b"RIFF....WAVE"), Some("wav"));
        assert_eq!(audio_format(b"ID3\x04"), Some("mp3"));
        assert_eq!(audio_format(&[0xff, 0xfb, 0x90]), Some("mp3"));
        assert_eq!(audio_format(b"{\"error\": \"nope\"}"), None);
        assert_eq!(audio_format(b""), None);
    }

    #[test]
    fn the_keys_of_the_answer_are_sentences() {
        assert!(describe(401).contains("rejected the key"));
        assert!(describe(429).contains("rate limited"));
        assert!(describe(404).contains("model or voice"));
        assert!(describe(503).contains("failed"));
        assert!(describe(418).contains("418"));
    }

    #[test]
    fn nothing_is_sent_when_there_is_nothing_to_say() {
        assert!(check_text("").is_err());
        assert!(check_text("   \n  ").is_err());
        // Counted in characters, not bytes: a 4,000-character reply in a language
        // outside Latin-1 is still a reply one request reads.
        let fits = "é".repeat(MAX_CHARS);
        assert_eq!(
            check_text(&fits)
                .expect("exactly at the cap")
                .chars()
                .count(),
            MAX_CHARS
        );
        let over = "a".repeat(MAX_CHARS + 1);
        assert!(check_text(&over).is_err());
        assert_eq!(check_text("  hello  ").expect("trimmed"), "hello");
    }
}
