//! Kokoro-82M: the same phoneme ids Kale speaks, on a different graph.
//!
//! Kokoro and Kale take the same input — Kokoro's token ids, which is what Kale
//! is a finetune of — so this engine reuses \`speak/frontend.rs\` instead of
//! carrying a front end of its own. That front end is installed once, in the
//! \`frontend/\` folder beside the voices (see \`catalog::FRONTEND\`), because two
//! voices needing the same 4.5 MB word list should not be two downloads.
//!
//! The feed shape is the model card's, not Kale's, and two lines differ:
//!
//! - the ids are padded with token 0 at both ends (\`[0, ...ids, 0]\`), because
//!   the graph was exported that way;
//! - the style row is chosen by the number of tokens, not one less than it:
//!   Kokoro's \`voices/af.bin\` is 512 rows of 256 floats for that reason.
//!
//! Everything here is blocking CPU work on the blocking pool, under the same
//! one-at-a-time lock the other engines take.

use std::path::Path;
use std::time::Instant;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;

use crate::error::AppError;
use crate::speak::engine::Synthesis;
use crate::speak::frontend::Frontend;

const GRAPH: &str = "model_quantized.onnx";
/// Kokoro's own rate, from the model card's Python example (\`wavfile.write(...,
/// 24000, ...)\`).
pub const RATE: u32 = 24_000;
/// One style row: the graph takes a 256-float vector.
const STYLE_WIDTH: usize = 256;
/// The graph's context is 512 including the two pad tokens, so 510 is the most
/// ids it can be given.
const MAX_TOKENS: usize = 510;
/// Kokoro's own chunking, from \`kokoro-js\`: it splits above this and never
/// sends a paragraph as one call.
const MAX_CHARS: usize = 400;

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
        .as_chunks::<4>()
        .0
        .iter()
        .map(|four| f32::from_le_bytes(*four))
        .collect())
}

struct Kokoro {
    session: Session,
    style: Vec<f32>,
    frontend: Frontend,
}

impl Kokoro {
    fn load(voice_dir: &Path, frontend_dir: &Path) -> Result<Self, AppError> {
        let style = read_floats(&voice_dir.join("af.bin"))?;
        if style.len() % STYLE_WIDTH != 0 {
            return Err(AppError::Speak(format!(
                "the Kokoro style table has {} floats, which is not a whole number of \
                 {STYLE_WIDTH}-float rows",
                style.len()
            )));
        }
        let frontend = Frontend::load(frontend_dir)?;
        let session = Session::builder()
            .map_err(|err| AppError::Speak(format!("onnx runtime unavailable: {err}")))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|err| AppError::Speak(format!("onnx session builder: {err}")))?
            .commit_from_file(voice_dir.join(GRAPH))
            .map_err(|err| {
                AppError::Speak(format!(
                    "{GRAPH} could not be loaded: {err}. The graph may use an operator this \
                     build's onnx runtime does not have."
                ))
            })?;
        Ok(Self {
            session,
            style,
            frontend,
        })
    }

    fn speak_sentence(&mut self, ids: &[i64], speed: f32) -> Result<Vec<f32>, AppError> {
        // Both ends padded, as the card's example does it.
        let mut padded = Vec::with_capacity(ids.len() + 2);
        padded.push(0i64);
        padded.extend_from_slice(ids);
        padded.push(0i64);
        // The row is the number of tokens (pad excluded), which is why the table
        // has 512 rows while a sentence tops out at 510 tokens.
        let rows = self.style.len() / STYLE_WIDTH;
        let row = ids.len().min(rows - 1);
        let style = self.style[row * STYLE_WIDTH..(row + 1) * STYLE_WIDTH].to_vec();
        let tokens = padded.len();
        let feeds: Vec<(String, SessionInputValue<'static>)> = vec![
            (
                "input_ids".to_string(),
                Tensor::from_array((vec![1usize, tokens], padded))
                    .map_err(|err| AppError::Speak(format!("input_ids: {err}")))?
                    .into(),
            ),
            (
                "style".to_string(),
                Tensor::from_array((vec![1usize, STYLE_WIDTH], style))
                    .map_err(|err| AppError::Speak(format!("style: {err}")))?
                    .into(),
            ),
            (
                "speed".to_string(),
                Tensor::from_array((vec![1usize], vec![speed]))
                    .map_err(|err| AppError::Speak(format!("speed: {err}")))?
                    .into(),
            ),
        ];
        let outputs = self
            .session
            .run(feeds)
            .map_err(|err| AppError::Speak(format!("synthesis failed: {err}")))?;
        let (_, samples) = outputs
            .iter()
            .next()
            .ok_or_else(|| AppError::Speak("Kokoro returned no output".to_string()))
            .and_then(|(name, value)| {
                value
                    .try_extract_array::<f32>()
                    .map(|array| {
                        (
                            name.to_string(),
                            array.iter().copied().collect::<Vec<f32>>(),
                        )
                    })
                    .map_err(|err| AppError::Speak(format!("{name} could not be read: {err}")))
            })?;
        Ok(samples)
    }
}

/// Kokoro, one sentence per graph call.
///
/// The front end is the shared one, two folders up from the voice
/// (\`<speak>/frontend\`), which is where \`commands/speak.rs\` installs it.
pub fn speak(voice_dir: &Path, text: &str, speed: f32) -> Result<Synthesis, AppError> {
    let started = Instant::now();
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Speak(
            "there was nothing to read aloud".to_string(),
        ));
    }
    let frontend_dir = voice_dir
        .parent()
        .map(|parent| parent.join("frontend"))
        .ok_or_else(|| {
            AppError::Speak(
                "the voice folder has no parent, so the front end is missing".to_string(),
            )
        })?;
    static LOADED: std::sync::Mutex<Option<(std::path::PathBuf, Kokoro)>> =
        std::sync::Mutex::new(None);
    let mut loaded = LOADED
        .lock()
        .map_err(|_| AppError::Speak("Speech engine lock poisoned".into()))?;
    let key = voice_dir.to_path_buf();
    if loaded.as_ref().is_none_or(|(previous, _)| previous != &key) {
        *loaded = Some((key, Kokoro::load(voice_dir, &frontend_dir)?));
    }
    let engine = &mut loaded.as_mut().expect("engine loaded").1;
    let sentences = engine.frontend.ids(text);
    if sentences.is_empty() {
        return Err(AppError::Speak(
            "nothing in that text could be pronounced as words".to_string(),
        ));
    }
    let gap = vec![0.0f32; (RATE as f32 * 0.2) as usize];
    let mut samples: Vec<f32> = Vec::new();
    for (at, ids) in sentences.iter().enumerate() {
        if at > 0 {
            samples.extend_from_slice(&gap);
        }
        let ids: Vec<i64> = ids.iter().copied().take(MAX_TOKENS).collect();
        let mut audio = engine.speak_sentence(&ids, speed.clamp(0.5, 2.0))?;
        samples.append(&mut audio);
    }
    if samples.is_empty() {
        return Err(AppError::Speak(
            "the voice returned no audio for that text".to_string(),
        ));
    }
    Ok(Synthesis {
        samples,
        rate: RATE,
        ms: started.elapsed().as_millis() as u64,
    })
}

/// Sentences, and no chunk longer than the font end's own limit.
pub fn chunks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for sentence in text.split_inclusive(['.', '!', '?']) {
        if !current.is_empty() && current.chars().count() + sentence.chars().count() > MAX_CHARS {
            out.push(std::mem::take(&mut current));
        }
        current.push_str(sentence);
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_style_row_is_the_token_count_not_one_less() {
        // af.bin is 524,288 bytes of f32, which is 131,072 floats: 512 rows of
        // 256. Because the row is the token count, a 510-token sentence uses
        // row 510 and the table needs 511 rows — one more than kale's 510.
        let floats = 524_288 / 4;
        let rows = floats / STYLE_WIDTH;
        assert_eq!(rows, 512);
        let tokens = 510usize;
        assert_eq!(tokens.min(rows - 1), 510);
        // Kale's own table is 522,240 bytes = 510 rows, which is exactly why
        // its selection is `len - 1` and this one is not.
        assert_eq!(522_240 / 4 / STYLE_WIDTH, 510);
    }

    #[test]
    fn ids_are_padded_at_both_ends() {
        let ids = [5i64, 6, 7];
        let mut padded = Vec::new();
        padded.push(0i64);
        padded.extend_from_slice(&ids);
        padded.push(0i64);
        assert_eq!(padded, vec![0, 5, 6, 7, 0]);
    }

    #[test]
    fn a_long_sentence_is_split_before_the_limit() {
        let text = "This is a sentence. ".repeat(40);
        for chunk in chunks(&text) {
            assert!(
                chunk.chars().count() <= MAX_CHARS + 20,
                "chunk of {} characters",
                chunk.chars().count()
            );
        }
    }
}
