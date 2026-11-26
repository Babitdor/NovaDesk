//! On-demand model loading and inference.
//!
//! One model is resident at a time. Loading is hundreds of megabytes and a few
//! hundred milliseconds, so the holder keeps the last engine alive and drops it
//! when the caller asks for a different model or the window sits idle.
//!
//! Everything here is synchronous: the command layer runs it on a blocking
//! thread, which is why the mutex can stay a plain std mutex.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use transcribe_rs::onnx::canary::CanaryModel;
use transcribe_rs::onnx::cohere::CohereModel;
use transcribe_rs::onnx::moonshine::{MoonshineModel, MoonshineVariant};
use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::sense_voice::SenseVoiceModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperLoadParams};
use transcribe_rs::{SpeechModel, TranscribeOptions};

use crate::error::AppError;
use crate::voice::catalog::{Engine, Model, Quant};

/// Drop the weights after this much silence. Five minutes keeps a dictation
/// session warm without holding a model for the whole life of the window.
const IDLE: Duration = Duration::from_secs(300);

struct Loaded {
    id: &'static str,
    model: Box<dyn SpeechModel + Send>,
    used: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub struct Transcription {
    pub text: String,
    pub ms: u64,
}

/// The process-wide model holder. One per app, behind AppState.
#[derive(Default)]
pub struct VoiceEngine {
    loaded: Mutex<Option<Loaded>>,
}

impl VoiceEngine {
    /// Transcribe 16 kHz mono samples with the given model.
    ///
    /// Refuses, rather than silently returning nothing, when the model is not
    /// installed, when the catalogue says it cannot run, or when the request
    /// carries no audio.
    pub fn transcribe(
        &self,
        model: &'static Model,
        dir: &Path,
        language: Option<&str>,
        samples: &[f32],
    ) -> Result<Transcription, AppError> {
        if !model.supported {
            return Err(AppError::Voice(
                model
                    .unsupported_reason
                    .unwrap_or("this model cannot run on this build")
                    .to_string(),
            ));
        }
        if model.required.iter().any(|name| !dir.join(name).is_file()) {
            return Err(AppError::Voice(format!(
                "{} is not installed; download it in Settings, Voice",
                model.name
            )));
        }
        if samples.is_empty() {
            return Err(AppError::Voice(
                "there was no audio to transcribe".to_string(),
            ));
        }

        let start = Instant::now();
        let mut guard = self
            .loaded
            .lock()
            .map_err(|_| AppError::Voice("the voice engine is unavailable".to_string()))?;
        let stale = match guard.as_ref() {
            Some(loaded) => loaded.id != model.id || loaded.used.elapsed() > IDLE,
            None => true,
        };
        if stale {
            // Free the old weights before reading the new ones so peak memory
            // stays at one model.
            *guard = None;
            *guard = Some(load(model, dir)?);
        }
        let loaded = guard.as_mut().expect("the model was just loaded");

        let options = TranscribeOptions {
            language: language.map(str::to_string),
            ..Default::default()
        };
        let result = loaded.model.transcribe(samples, &options).map_err(|err| {
            AppError::Voice(format!("{} could not transcribe: {err}", model.name))
        })?;
        loaded.used = Instant::now();

        Ok(Transcription {
            text: result.text.trim().to_string(),
            ms: start.elapsed().as_millis() as u64,
        })
    }
}

/// The file suffix the engine looks for, taken from the catalogue rather than
/// guessed per engine: an int8 download is useless if the loader expects fp32.
fn precision(quant: Quant) -> Quantization {
    match quant {
        Quant::Fp32 => Quantization::FP32,
        Quant::Int8 => Quantization::Int8,
        Quant::Int4 => Quantization::Int4,
    }
}

fn load(model: &'static Model, dir: &Path) -> Result<Loaded, AppError> {
    let quant = precision(model.quant);
    let engine: Box<dyn SpeechModel + Send> = match model.engine {
        Engine::Parakeet => {
            Box::new(ParakeetModel::load(dir, &quant).map_err(|err| load_failure(model, err))?)
        }
        Engine::Canary => {
            Box::new(CanaryModel::load(dir, &quant).map_err(|err| load_failure(model, err))?)
        }
        Engine::Cohere => {
            Box::new(CohereModel::load(dir, &quant).map_err(|err| load_failure(model, err))?)
        }
        Engine::MoonshineBase => Box::new(
            MoonshineModel::load(dir, MoonshineVariant::Base, &quant)
                .map_err(|err| load_failure(model, err))?,
        ),
        Engine::MoonshineTiny => Box::new(
            MoonshineModel::load(dir, MoonshineVariant::Tiny, &quant)
                .map_err(|err| load_failure(model, err))?,
        ),
        Engine::SenseVoice => {
            Box::new(SenseVoiceModel::load(dir, &quant).map_err(|err| load_failure(model, err))?)
        }
        Engine::Whisper => {
            // whisper.cpp takes one GGML file rather than a directory, and the
            // quantisation is baked into that file. CPU only, matching the ONNX
            // path: asking for a GPU here would add a failure mode on machines
            // that have none, and the flag is a later decision.
            Box::new(
                WhisperEngine::load_with_params(
                    &dir.join(model.required[0]),
                    WhisperLoadParams {
                        use_gpu: false,
                        flash_attn: false,
                        ..Default::default()
                    },
                )
                .map_err(|err| load_failure(model, err))?,
            )
        }
        Engine::None => {
            return Err(AppError::Voice(format!(
                "{} has no inference engine",
                model.name
            )))
        }
    };
    Ok(Loaded {
        id: model.id,
        model: engine,
        used: Instant::now(),
    })
}

/// Loading fails for a short list of reasons; the one users hit is RAM, so the
/// message names the model's size instead of repeating an ONNX stack trace.
fn load_failure(model: &'static Model, err: transcribe_rs::TranscribeError) -> AppError {
    AppError::Voice(format!(
        "{} could not be loaded ({err}). It needs about {} MB of free memory while it runs.",
        model.name,
        model.bytes / 1_000_000
    ))
}
