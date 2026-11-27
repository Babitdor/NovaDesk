//! Speech commands: what Nova can read aloud, its downloads, and the synthesis
//! itself.
//!
//! The download path is the same one the dictation models use — one file at a
//! time, exact sizes, manifest last — plus one extra step: a Piper voice is
//! useless without Piper itself, so the first Piper voice installs the runtime
//! ahead of the weights and reports that as its own phase. A voice whose engine
//! runs in process (Kale, commands/speak.rs -> speak/onnx.rs) skips that phase
//! entirely, since it has no runtime to wait for.

use std::path::PathBuf;

use base64::Engine as _;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::speak::catalog::{self, VoiceView, FRONTEND, RUNTIME};
use crate::speak::service::{self, Profile};
use crate::voice::download::{self, phase, Progress, Spec};
use crate::AppState;

/// The folder Piper lives in, inside Speak's own folder.
const RUNTIME_DIR: &str = "piper";
const RUNTIME_EXE: &str = "piper.exe";
/// The folder the shared phoneme front end lives in: one copy, used by every
/// voice whose graph reads phonemes rather than letters.
const FRONTEND_DIR: &str = "frontend";

/// <app data>/speak — Piper, one folder per voice, and the scratch wave.
fn root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| AppError::Speak(format!("could not resolve the voice folder: {err}")))?;
    Ok(dir.join("speak"))
}

fn voice_dir(app: &AppHandle, id: &str) -> Result<PathBuf, AppError> {
    Ok(root(app)?.join(id))
}

fn runtime_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(root(app)?.join(RUNTIME_DIR))
}

fn runtime_exe(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(runtime_dir(app)?.join(RUNTIME_EXE))
}

/// Whether Piper's files are all on disk. A half-unpacked archive fails this,
/// which is the point of checking the phoneme folder too.
fn runtime_ready(app: &AppHandle) -> Result<bool, AppError> {
    let dir = runtime_dir(app)?;
    Ok(download::archive_ready(&dir, RUNTIME.files, RUNTIME.dirs))
}

fn frontend_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(root(app)?.join(FRONTEND_DIR))
}

/// Whether the shared phoneme front end is fully on disk. A half-installed one
/// fails this, which is the point of checking every name.
fn frontend_ready(app: &AppHandle) -> Result<bool, AppError> {
    let dir = frontend_dir(app)?;
    Ok(download::archive_ready(
        &dir,
        catalog::FRONTEND.required,
        &[],
    ))
}

fn voice_ready(app: &AppHandle, voice: &catalog::Voice) -> Result<bool, AppError> {
    let dir = voice_dir(app, voice.id)?;
    Ok(voice.required.iter().all(|name| dir.join(name).is_file()))
}

#[tauri::command]
pub fn speak_catalog() -> Vec<VoiceView> {
    catalog::VOICES.iter().map(catalog::Voice::view).collect()
}

/// Voices with their weights on disk. The runtime is reported separately, so the
/// page can say "installing Piper" rather than blaming the voice.
#[tauri::command]
pub fn speak_installed(app: AppHandle) -> Result<Vec<String>, AppError> {
    let mut present = Vec::new();
    for voice in catalog::VOICES {
        if voice_ready(&app, voice)? {
            present.push(voice.id.to_string());
        }
    }
    Ok(present)
}

#[tauri::command]
pub fn speak_runtime_ready(app: AppHandle) -> Result<bool, AppError> {
    runtime_ready(&app)
}

/// Streams Piper (once) and then the voice into the app data folder, reporting
/// bytes as they land. Resolves when the voice can be spoken.
#[tauri::command]
pub async fn speak_download(
    app: AppHandle,
    id: String,
    on_event: Channel<Progress>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let voice = catalog::find(&id)
        .ok_or_else(|| AppError::Speak(format!("Nova does not know a voice called {id}")))?;
    if !voice.supported {
        return Err(AppError::Speak(
            voice
                .unsupported_reason
                .unwrap_or("this voice cannot run on this build")
                .to_string(),
        ));
    }

    let cancel = CancellationToken::new();
    // One writer per voice, for the same reason as the dictation models: a
    // second transfer writes into the same folder as the first.
    if state
        .speak_downloads
        .lock()
        .expect("speech downloads mutex poisoned")
        .insert(id.clone(), cancel.clone())
        .is_some()
    {
        return Err(AppError::Speak("this voice is already downloading".into()));
    }

    let result = download_voice(&app, voice, &cancel, &on_event).await;

    state
        .speak_downloads
        .lock()
        .expect("speech downloads mutex poisoned")
        .remove(&id);
    result
}

async fn download_voice(
    app: &AppHandle,
    voice: &'static catalog::Voice,
    cancel: &CancellationToken,
    on: &Channel<Progress>,
) -> Result<(), AppError> {
    // Piper itself is installed by the first Piper voice and by no other: an
    // ONNX voice brings everything it needs in its own folder.
    if voice.engine == catalog::TtsEngine::Piper && !runtime_ready(app)? {
        let (url, bytes) = match RUNTIME.source {
            catalog::Source::Zip(url, bytes) => (url, bytes),
            catalog::Source::Files(_) => {
                return Err(AppError::Speak(
                    "the Piper bundle is not described as one archive".to_string(),
                ))
            }
        };
        let dir = runtime_dir(app)?;
        download::install_zip(url, bytes, &dir, cancel, on, phase::RUNTIME).await?;
        if !runtime_ready(app)? {
            return Err(AppError::Speak(
                "Piper finished downloading but its files are incomplete".to_string(),
            ));
        }
    }
    // A voice whose graph takes phonemes needs the shared front end, installed
    // once for all of them, the same way the first Piper voice needs Piper.
    if voice.engine.needs_frontend() && !frontend_ready(app)? {
        let dir = frontend_dir(app)?;
        let spec = Spec {
            id: FRONTEND_DIR,
            files: FRONTEND.files,
            bytes: FRONTEND.bytes,
            required: FRONTEND.required,
            engine: FRONTEND_DIR,
        };
        download::install_spec(&spec, &dir, cancel, on, phase::MODEL).await?;
        if !frontend_ready(app)? {
            return Err(AppError::Speak(
                "the phoneme front end finished downloading but its files are incomplete"
                    .to_string(),
            ));
        }
    }
    if voice_ready(app, voice)? {
        return Ok(());
    }
    let dir = voice_dir(app, voice.id)?;
    let spec = Spec {
        id: voice.id,
        files: voice.files(),
        bytes: voice.bytes,
        required: voice.required,
        engine: voice.engine.name(),
    };
    download::install_spec(&spec, &dir, cancel, on, phase::VOICE).await
}

#[tauri::command]
pub fn speak_cancel(id: String, state: State<'_, AppState>) -> bool {
    stop(&state, &id)
}

fn stop(state: &State<'_, AppState>, id: &str) -> bool {
    match state
        .speak_downloads
        .lock()
        .expect("speech downloads mutex poisoned")
        .get(id)
        .cloned()
    {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

/// Removes one voice's weights. Piper itself stays: another voice may use it,
/// and re-downloading 22 MB to delete one voice would be rude.
#[tauri::command]
pub async fn speak_delete(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if catalog::find(&id).is_none() {
        return Err(AppError::Speak(format!(
            "Nova does not know a voice called {id}"
        )));
    }
    // A live download recreates the folder as it streams, so stop it and let
    // its task leave the map before removing what it owns.
    // shortcut: the wait is capped at 5 s; a wedged transfer still gets the
    // folder removed, and its leftover is overwritten by the next install.
    stop(&state, &id);
    for _ in 0..50 {
        let live = state
            .speak_downloads
            .lock()
            .expect("speech downloads mutex poisoned")
            .contains_key(&id);
        if !live {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let dir = voice_dir(&app, &id)?;
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::from_io(err, &dir)),
    }
}

/// One synthesized reply.
///
/// `audio` is base64 either of little-endian f32 samples at `sampleRate` (the
/// on-device path) or of the file the service returned, and `kind` says which.
/// The page turns both into an AudioBuffer or a blob and plays them.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechAudio {
    pub audio: String,
    pub kind: &'static str,
    pub sample_rate: u32,
    pub ms: u64,
}

/// One request, grouped rather than spread over eight parameters: the shape is
/// the page's, and it is the same shape for both destinations.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakRequest {
    destination: String,
    model_id: Option<String>,
    profile: Option<Profile>,
    text: String,
    speed: Option<f32>,
    speaker: Option<u32>,
}

#[tauri::command]
pub async fn speak_synthesize(
    app: AppHandle,
    request: SpeakRequest,
    state: State<'_, AppState>,
) -> Result<SpeechAudio, AppError> {
    let SpeakRequest {
        destination,
        model_id,
        profile,
        text,
        speed,
        speaker,
    } = request;
    let speed = speed.unwrap_or(1.0);
    match destination.as_str() {
        "local" => {
            let id = model_id.unwrap_or_default();
            let voice = catalog::find(&id).ok_or_else(|| {
                AppError::Speak(format!("Nova does not know a voice called {id}"))
            })?;
            // Only a Piper voice needs Piper. An in-process ONNX voice is
            // nothing but its own files, and refusing it because the Piper
            // bundle is absent would make the smallest voice here the hardest
            // one to run.
            if voice.engine == catalog::TtsEngine::Piper && !runtime_ready(&app)? {
                return Err(AppError::Speak(
                    "Piper is not installed; download a voice in Settings, Voice and it arrives with it"
                        .to_string(),
                ));
            }
            // A phoneme voice's graph is useless without the shared front end,
            // so say that rather than failing at the first file read.
            if voice.engine.needs_frontend() && !frontend_ready(&app)? {
                return Err(AppError::Speak(format!(
                    "{} needs the shared phoneme front end, which is not installed; download it \
                     again in Settings, Voice and it arrives with the voice",
                    voice.name
                )));
            }
            let dir = voice_dir(&app, voice.id)?;
            let exe = runtime_exe(&app)?;
            let engine = state.speak.clone();
            let synthesis = engine
                .synthesize(voice, &dir, &exe, &text, speaker, speed)
                .await?;
            Ok(SpeechAudio {
                audio: encode_samples(&synthesis.samples),
                kind: "samples",
                sample_rate: synthesis.rate,
                ms: synthesis.ms,
            })
        }
        "service" => {
            let profile = profile
                .ok_or_else(|| AppError::Speak("no speech service is selected".to_string()))?;
            let audio = service::synthesize(&profile, &text, speed).await?;
            Ok(SpeechAudio {
                audio: base64::engine::general_purpose::STANDARD.encode(&audio.bytes),
                kind: audio.format,
                // A service returns compressed audio; the page decodes it itself.
                sample_rate: 0,
                ms: audio.ms,
            })
        }
        other => Err(AppError::Speak(format!(
            "Nova cannot read aloud through {other}"
        ))),
    }
}

/// f32 samples as base64, little endian, mono. The same transport the dictation
/// path uses in the other direction.
fn encode_samples(samples: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_go_over_as_little_endian_f32() {
        let encoded = encode_samples(&[0.0, 0.5, -0.5]);
        let raw = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .expect("valid base64");
        assert_eq!(raw.len(), 12);
        let first = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        assert_eq!(first, 0.0);
        let second = f32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
        assert_eq!(second, 0.5);
    }

    #[test]
    fn nothing_is_encoded_for_an_empty_reply() {
        assert_eq!(encode_samples(&[]), "");
    }

    #[test]
    fn the_runtime_binary_is_inside_the_runtime_folder() {
        // The layout is what engine.rs is handed: the exe path and the voice
        // folder are siblings under speak/.
        let root = std::path::Path::new("/speak");
        assert_eq!(
            root.join(RUNTIME_DIR).join(RUNTIME_EXE),
            std::path::Path::new("/speak/piper/piper.exe")
        );
    }

    #[test]
    fn an_unknown_voice_is_refused_by_name() {
        assert!(catalog::find("no-such-voice").is_none());
    }
}
