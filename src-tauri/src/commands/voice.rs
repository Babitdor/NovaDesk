//! Voice commands: catalogue, downloads and transcription.

use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::voice::audio;
use crate::voice::catalog::{self, ModelView};
use crate::voice::download::{self, Progress};
use crate::voice::engine::Transcription;
use crate::AppState;

#[tauri::command]
pub fn voice_catalog() -> Vec<ModelView> {
    catalog::MODELS.iter().map(catalog::Model::view).collect()
}

#[tauri::command]
pub fn voice_installed(app: AppHandle) -> Result<Vec<String>, AppError> {
    download::installed(&app)
}

/// Streams a model into the app data folder, reporting bytes as they land.
/// Resolves when the model is installed or the transfer failed.
#[tauri::command]
pub async fn voice_download(
    app: AppHandle,
    id: String,
    on_event: Channel<Progress>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let cancel = CancellationToken::new();
    state
        .voice_downloads
        .lock()
        .expect("voice downloads mutex poisoned")
        .insert(id.clone(), cancel.clone());

    let result = download::install(&app, &id, &cancel, &on_event).await;

    state
        .voice_downloads
        .lock()
        .expect("voice downloads mutex poisoned")
        .remove(&id);
    result
}

#[tauri::command]
pub fn voice_cancel(id: String, state: State<'_, AppState>) -> bool {
    stop(&state, &id)
}

fn stop(state: &State<'_, AppState>, id: &str) -> bool {
    match state
        .voice_downloads
        .lock()
        .expect("voice downloads mutex poisoned")
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

#[tauri::command]
pub async fn voice_delete(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    // A live download would recreate the folder as it streams, so stop it first.
    stop(&state, &id);
    download::remove(&app, &id).await
}

/// Transcribe one recording.
///
/// "samples" is base64 of little-endian f32, interleaved at the device rate;
/// the downmix and the resample to 16 kHz happen here, on a blocking thread,
/// because that is where they can be tested.
#[tauri::command]
pub async fn voice_transcribe(
    app: AppHandle,
    model_id: String,
    language: Option<String>,
    samples: String,
    sample_rate: u32,
    channels: u16,
    state: State<'_, AppState>,
) -> Result<Transcription, AppError> {
    let model = catalog::find(&model_id)
        .ok_or_else(|| AppError::Voice(format!("Nova does not know a model called {model_id}")))?;
    if !model.supported {
        return Err(AppError::Voice(
            model
                .unsupported_reason
                .unwrap_or("this model cannot run on this build")
                .to_string(),
        ));
    }
    let dir = download::model_dir(&app, model.id)?;
    let samples = audio::decode_ipc(&samples, channels, sample_rate)?;
    let language = language.filter(|code| !code.is_empty() && !code.eq_ignore_ascii_case("all"));

    let engine = state.voice.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.transcribe(model, &dir, language.as_deref(), &samples)
    })
    .await
    .map_err(|err| AppError::Voice(format!("the transcription task failed: {err}")))?
}

/// Whether ONNX Runtime actually starts in this process.
///
/// The runtime is linked into the executable, so there is no dll to look for;
/// asking ort for its global environment is what makes the library initialise,
/// and a failure there means every transcription in this build will fail. The
/// answer is cached: the environment is a process-wide singleton.
#[tauri::command]
pub fn voice_engine_ready() -> bool {
    static READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *READY.get_or_init(|| ort::environment::Environment::current().is_ok())
}
