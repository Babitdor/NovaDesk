//! The in-process ONNX engines: weights that run inside this binary on ort.
//!
//! Piper stays a child process (espeak-ng is GPLv3 and shipping it inside this
//! executable would make the executable GPLv3). A voice whose weights are plain
//! ONNX has no such constraint, so it runs here instead: one session per
//! synthesis, built on the blocking pool, under the same lock Piper takes. That
//! keeps one reply at a time, which matters more for an in-process engine than
//! for a child process: two sessions would each get half the cores.
//!
//! Everything here is synchronous CPU work on purpose. The command layer hands
//! it to tokio::task::spawn_blocking, so a 15 MB graph and a few hundred ms of
//! inference never sits on the async runtime.

use std::path::Path;
use std::time::Instant;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use serde_json::Value as Json;

use crate::error::AppError;
use crate::speak::engine::Synthesis;
use crate::speak::frontend::{Frontend, GAP_SECONDS};

/// The quantised graph: 14,767,138 bytes, checked against the file's published
/// size before it is downloaded, so this name and that number agree.
const KALE_GRAPH: &str = "kale-tts-1-nano.onnx";
/// Kokoro's rate. The model's own runtime states it (SAMPLE_RATE in
/// js/kale-tts.mjs) and the graph does not carry it, so it is a constant here
/// rather than something read from the weights.
const KALE_RATE: u32 = 24_000;
/// One style row: the graph takes a 256-float vector, one per transcript
/// length, chosen by the number of tokens.
const STYLE_WIDTH: usize = 256;

/// One voice preset: a style table and the per-voice inputs the graph wants.
struct Preset {
    id: String,
    name: String,
}

/// One extra graph input, straight from voices.json: its name, its shape and
/// the values that the voice's .extra.bin carries for it.
struct Extra {
    name: String,
    dims: Vec<usize>,
    values: Vec<f32>,
}

fn read_json(path: &Path) -> Result<Json, AppError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| AppError::Speak(format!("{} could not be read: {err}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|err| AppError::Speak(format!("{} is not valid JSON: {err}", path.display())))
}

/// f32 samples from a file of little-endian f32. The style tables and the voice
/// inputs ship that way, which is why they need no parser of their own.
fn read_floats(path: &Path) -> Result<Vec<f32>, AppError> {
    let bytes = std::fs::read(path)
        .map_err(|err| AppError::Speak(format!("{} could not be read: {err}", path.display())))?;
    if bytes.len() % 4 != 0 {
        return Err(AppError::Speak(format!(
            "{} is {} bytes, which is not a whole number of floats",
            path.display(),
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]]))
        .collect())
}

fn presets(manifest: &Json) -> Result<Vec<Preset>, AppError> {
    let listed = manifest
        .get("voices")
        .and_then(Json::as_array)
        .ok_or_else(|| AppError::Speak("voices.json lists no voices for this model".to_string()))?;
    let mut out = Vec::with_capacity(listed.len());
    for voice in listed {
        let id = voice.get("id").and_then(Json::as_str).unwrap_or_default();
        if id.is_empty() {
            return Err(AppError::Speak(
                "voices.json has a voice with no id".to_string(),
            ));
        }
        let name = voice
            .get("name")
            .and_then(Json::as_str)
            .unwrap_or(id)
            .to_string();
        out.push(Preset {
            id: id.to_string(),
            name,
        });
    }
    Ok(out)
}

fn extras(manifest: &Json, dir: &Path, id: &str) -> Result<Vec<Extra>, AppError> {
    let listed = manifest
        .get("voice_inputs")
        .and_then(Json::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if listed.is_empty() {
        return Ok(Vec::new());
    }
    let path = dir.join(format!("{id}.extra.bin"));
    let floats = read_floats(&path)?;
    let mut at = 0;
    let mut out = Vec::with_capacity(listed.len());
    for input in listed {
        let name = input.get("name").and_then(Json::as_str).unwrap_or_default();
        let dims: Vec<usize> = input
            .get("dims")
            .and_then(Json::as_array)
            .map(|dims| {
                dims.iter()
                    .filter_map(Json::as_u64)
                    .map(|dim| dim as usize)
                    .collect()
            })
            .unwrap_or_default();
        let count: usize = dims.iter().product();
        let values = floats.get(at..at + count).ok_or_else(|| {
            AppError::Speak(format!(
                "{} holds {} floats, and voices.json wants more than that for {name}",
                path.display(),
                floats.len()
            ))
        })?;
        at += count;
        out.push(Extra {
            name: name.to_string(),
            dims,
            values: values.to_vec(),
        });
    }
    if at != floats.len() {
        return Err(AppError::Speak(format!(
            "{} carries {} floats nothing in voices.json asks for",
            path.display(),
            floats.len() - at
        )));
    }
    Ok(out)
}

/// One synthesis, ready to run: the session, the voice picked and the reader.
struct Kale {
    session: Session,
    output: String,
    style: Vec<f32>,
    extras: Vec<Extra>,
    frontend: Frontend,
}

impl Kale {
    fn load(dir: &Path, speaker: Option<u32>) -> Result<Self, AppError> {
        let manifest = read_json(&dir.join("voices.json"))?;
        let presets = presets(&manifest)?;
        let index = speaker.unwrap_or(0) as usize;
        let preset = presets.get(index).ok_or_else(|| {
            AppError::Speak(format!(
                "this voice has {} speakers, and speaker {} was asked for",
                presets.len(),
                index
            ))
        })?;
        let style = read_floats(&dir.join(format!("{}.bin", preset.id)))?;
        if style.len() % STYLE_WIDTH != 0 {
            return Err(AppError::Speak(format!(
                "the style table for {} has {} floats, which is not a whole number of {STYLE_WIDTH}-float rows",
                preset.name,
                style.len()
            )));
        }
        let extras = extras(&manifest, dir, &preset.id)?;
        let frontend = Frontend::load(dir)?;
        // Level 3 is ONNX Runtime's full fusion pass. These graphs are a single
        // session each, so the cost is paid once per reply and is small next to
        // the inference itself.
        let session = Session::builder()
            .map_err(|err| AppError::Speak(format!("onnx runtime unavailable: {err}")))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|err| AppError::Speak(format!("onnx session builder: {err}")))?
            .commit_from_file(dir.join(KALE_GRAPH))
            .map_err(|err| {
                AppError::Speak(format!(
                    "{KALE_GRAPH} could not be loaded: {err}. The graph is newer than the \
                     onnx runtime this build links, so an operator it uses may be missing."
                ))
            })?;
        let output = session
            .outputs()
            .first()
            .map(|outlet| outlet.name().to_string())
            .ok_or_else(|| AppError::Speak(format!("{KALE_GRAPH} has no outputs")))?;
        Ok(Self {
            session,
            output,
            style,
            extras,
            frontend,
        })
    }

    /// Every graph input for one sentence. A graph input this engine does not
    /// know is a refusal with its name, not a silent skip: onnx runtime would
    /// otherwise run the session with a missing input and fail less clearly.
    ///
    /// The values own their memory, so the feeds carry no borrow of the session
    /// that built them: the session has to be mutated to run, and a borrow held
    /// in the feeds would make that impossible.
    fn feeds(
        session: &Session,
        style_table: &[f32],
        extras_table: &[Extra],
        ids: &[i64],
    ) -> Result<Vec<(String, SessionInputValue<'static>)>, AppError> {
        let rows = style_table.len() / STYLE_WIDTH;
        let row = ids.len().min(rows.max(1)) - 1;
        let style = style_table[row * STYLE_WIDTH..(row + 1) * STYLE_WIDTH].to_vec();
        let names: Vec<String> = session
            .inputs()
            .iter()
            .map(|input| input.name().to_string())
            .collect();
        let mut feeds = Vec::with_capacity(names.len());
        for name in names {
            let value: ort::session::SessionInputValue = match name.as_str() {
                "input_ids" => Tensor::from_array((vec![1usize, ids.len()], ids.to_vec()))
                    .map_err(|err| AppError::Speak(format!("input_ids tensor: {err}")))?
                    .into(),
                "style" => Tensor::from_array((vec![1usize, STYLE_WIDTH], style.clone()))
                    .map_err(|err| AppError::Speak(format!("style tensor: {err}")))?
                    .into(),
                // The model's own runtime pins this at 1.0: pace comes from the
                // graph, so the speed control does not reach this engine.
                "speed" => Tensor::from_array((vec![1usize], vec![1.0f32]))
                    .map_err(|err| AppError::Speak(format!("speed tensor: {err}")))?
                    .into(),
                other => {
                    let extra = extras_table.iter().find(|extra| extra.name == other);
                    let extra = extra.ok_or_else(|| {
                        AppError::Speak(format!(
                            "the graph wants an input called {other}, which voices.json does not \
                             describe for this voice"
                        ))
                    })?;
                    Tensor::from_array((extra.dims.clone(), extra.values.clone()))
                        .map_err(|err| AppError::Speak(format!("{other} tensor: {err}")))?
                        .into()
                }
            };
            feeds.push((name, value));
        }
        Ok(feeds)
    }

    fn speak_sentence(&mut self, ids: &[i64]) -> Result<Vec<f32>, AppError> {
        let feeds = Self::feeds(&self.session, &self.style, &self.extras, ids)?;
        let outputs = self
            .session
            .run(feeds)
            .map_err(|err| AppError::Speak(format!("synthesis failed: {err}")))?;
        let output = outputs
            .get(self.output.as_str())
            .ok_or_else(|| AppError::Speak(format!("the graph returned no {}", self.output)))?;
        let audio = output.try_extract_array::<f32>().map_err(|err| {
            AppError::Speak(format!("the returned audio could not be read: {err}"))
        })?;
        Ok(audio.iter().copied().collect())
    }
}

/// kale-tts-1-nano: Kokoro's architecture in a 14.8 MB graph, spoken from the
/// phoneme ids the front end produces.
///
/// A sentence is one graph call, as the model's own runtime does it, and the
/// calls are joined with a short silence so one sentence does not start inside
/// the tail of the one before it.
pub fn kale(
    dir: &Path,
    text: &str,
    speaker: Option<u32>,
    speed: f32,
) -> Result<Synthesis, AppError> {
    let started = Instant::now();
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Speak(
            "there was nothing to read aloud".to_string(),
        ));
    }
    let mut engine = Kale::load(dir, speaker)?;
    let sentences = engine.frontend.ids(text);
    if sentences.is_empty() {
        return Err(AppError::Speak(
            "nothing in that text could be pronounced as words".to_string(),
        ));
    }
    let gap = vec![0.0f32; (GAP_SECONDS * KALE_RATE as f32) as usize];
    let mut samples: Vec<f32> = Vec::new();
    for (at, sentence) in sentences.iter().enumerate() {
        if at > 0 {
            samples.extend_from_slice(&gap);
        }
        let mut audio = engine.speak_sentence(sentence)?;
        samples.append(&mut audio);
    }
    if samples.is_empty() {
        return Err(AppError::Speak(
            "the voice returned no audio for that text".to_string(),
        ));
    }
    // Speed is the one control this engine cannot honour without changing the
    // speaker's pitch, because the graph has no length-scale input. Saying so
    // in the code beats pretending: the page keeps the control for Piper, and
    // this branch is where its absence is visible.
    let _ = speed;
    Ok(Synthesis {
        samples,
        rate: KALE_RATE,
        ms: started.elapsed().as_millis() as u64,
    })
}
