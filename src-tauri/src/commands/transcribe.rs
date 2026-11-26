//! Hosted speech-to-text commands: the provider table, and one transcription
//! through a saved profile.
//!
//! The samples arrive exactly as they do for the on-device path, so the decode
//! and the resample are the same code (voice::audio); what differs is where the
//! audio goes afterwards.

use crate::error::AppError;
use crate::transcribe::service::{self, Profile, ProviderInfo};
use crate::voice::audio;
use crate::voice::engine::Transcription;

/// Where a hosted service can be pointed, and the models it publishes.
#[tauri::command]
pub fn transcribe_providers() -> &'static [ProviderInfo] {
    service::PROVIDERS
}

/// Transcribe one recording through a saved service.
#[tauri::command]
pub async fn voice_transcribe_service(
    profile: Profile,
    language: Option<String>,
    samples: String,
    sample_rate: u32,
    channels: u16,
) -> Result<Transcription, AppError> {
    let samples = audio::decode_ipc(&samples, channels, sample_rate)?;
    let language = language.filter(|code| !code.is_empty() && !code.eq_ignore_ascii_case("all"));
    service::transcribe(&profile, &samples, audio::TARGET_RATE, language.as_deref()).await
}
