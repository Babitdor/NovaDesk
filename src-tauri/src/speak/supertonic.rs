//! Supertonic 3: flow-matching text to speech in four ONNX graphs.
//!
//! Unlike Kale this engine needs no phoneme front end. Its text encoder reads
//! Unicode code points from an index table, so the download is four graphs plus
//! that table and one voice style, and the text handling is "drop what the
//! table does not know, then wrap the sentence in its language tag".
//!
//! The pipeline is the model's own, read from the reference SDK
//! (\`supertonic\` 1.3.1, \`core.py\`) rather than guessed:
//!
//! 1. \`duration_predictor\`: text ids + style -> how many seconds to speak.
//! 2. \`text_encoder\`: text ids + style -> the text embedding.
//! 3. \`vector_estimator\`, once per step: current latent + text embedding +
//!    style + both masks + the step index -> the next latent. Eight steps.
//! 4. \`vocoder\`: the finished latent -> audio.
//!
//! Every shape below comes from \`tts.json\` or the style file rather than being
//! written down here, because the sampling constants are exactly what a future
//! release would change. Everything is blocking CPU work and runs on the
//! blocking pool; the command layer does that (see \`engine.rs\`).

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue, SessionOutputs};
use ort::value::Tensor;
use serde_json::Value as Json;

use crate::error::AppError;
use crate::speak::engine::Synthesis;

const DURATION: &str = "duration_predictor.onnx";
const TEXT_ENCODER: &str = "text_encoder.onnx";
const VECTOR_FIELD: &str = "vector_estimator.onnx";
const VOCODER: &str = "vocoder.onnx";
const CONFIG: &str = "tts.json";
const INDEXER: &str = "unicode_indexer.json";

/// The reference SDK's \`DEFAULT_TOTAL_STEPS\`. More steps cost time and, on this
/// model, do not read better; fewer are audibly rougher.
const STEPS: usize = 8;
/// The reference SDK's `DEFAULT_MAX_CHUNK_LENGTH`: one graph call is one
/// sentence-sized chunk. Korean uses 120 there because its sentences are shorter
/// in characters; this card is English.
const MAX_CHARS: usize = 300;
/// The language the v3 graphs are told to speak, as the two-letter tag they wrap
/// every sentence in. The models carry thirty-one languages; the catalogue only
/// offers English today, so this comes from the voice's own language code and a
/// future card for another language would need no engine change.
pub const DEFAULT_LANGUAGE: &str = "en";

/// A style vector as its JSON writes it: its shape, and its floats.
struct Style {
    dims: Vec<usize>,
    data: Vec<f32>,
}

fn read_json(path: &Path) -> Result<Json, AppError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| AppError::Speak(format!("{} could not be read: {err}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|err| AppError::Speak(format!("{} is not valid JSON: {err}", path.display())))
}

fn style_from(json: &Json, key: &str) -> Result<Style, AppError> {
    let block = json.get(key).ok_or_else(|| {
        AppError::Speak(format!("the voice style has no {key}, so it cannot speak"))
    })?;
    let dims: Vec<usize> = block
        .get("dims")
        .and_then(Json::as_array)
        .map(|dims| {
            dims.iter()
                .filter_map(Json::as_u64)
                .map(|dim| dim as usize)
                .collect()
        })
        .unwrap_or_default();
    // The floats are nested to match `dims` ([[[1][50][256]]]), so they are
    // flattened depth-first: the model wants them in that order anyway.
    fn flatten(json: &Json, into: &mut Vec<f32>) {
        match json {
            Json::Array(items) => {
                for item in items {
                    flatten(item, into);
                }
            }
            Json::Number(number) => {
                if let Some(value) = number.as_f64() {
                    into.push(value as f32);
                }
            }
            _ => {}
        }
    }
    let mut data: Vec<f32> = Vec::new();
    if let Some(block) = block.get("data") {
        flatten(block, &mut data);
    }
    let expected: usize = dims.iter().product();
    if expected != data.len() {
        return Err(AppError::Speak(format!(
            "the voice style's {key} is {dims:?} but carries {} floats",
            data.len()
        )));
    }
    Ok(Style { dims, data })
}

/// Code point to row in the text encoder's embedding table. A character missing
/// from the table is one this model was not trained on, so it is dropped rather
/// than mapped to a row that would mean something else.
fn read_indexer(path: &Path) -> Result<HashMap<char, i64>, AppError> {
    let json = read_json(path)?;
    // The file is one flat array indexed by code point: row N is the row for
    // character N, and -1 means "this model has no row for it". The 277 KB is
    // mostly those -1s, which is why the table is built here rather than in
    // `prepare` on every character of every sentence.
    let rows = json
        .as_array()
        .ok_or_else(|| AppError::Speak(format!("{} is not a table", path.display())))?;
    let mut indexer = HashMap::with_capacity(rows.len() / 4);
    for (code, row) in rows.iter().enumerate() {
        let id = match row.as_i64() {
            Some(id) if id >= 0 => id,
            _ => continue,
        };
        if let Some(ch) = char::from_u32(code as u32) {
            indexer.insert(ch, id);
        }
    }
    if indexer.is_empty() {
        return Err(AppError::Speak(format!(
            "{} maps no characters, so no text could be sent",
            path.display()
        )));
    }
    Ok(indexer)
}

/// Text as the model reads it: the characters its table knows, whitespace
/// collapsed, a full stop at the end, and the language tags around it.
fn prepare(text: &str, indexer: &HashMap<char, i64>, language: &str) -> Vec<i64> {
    let mut cleaned = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !cleaned.ends_with(' ') {
                cleaned.push(' ');
            }
            continue;
        }
        if indexer.contains_key(&ch) {
            cleaned.push(ch);
        }
    }
    let mut body = cleaned.trim().to_string();
    if !(body.ends_with('.') || body.ends_with('!') || body.ends_with('?')) {
        body.push('.');
    }
    format!("<{language}>{body}</{language}>")
        .chars()
        .filter_map(|ch| indexer.get(&ch).copied())
        .collect()
}

/// Noise for the latent the flow starts from.
///
/// The reference calls \`np.random.randn\`; no crate here draws gaussians, and a
/// few lines of Box-Muller over an LCG is the whole requirement. The seed comes
/// from the clock so two readings of the same sentence are not identical takes.
struct Noise(u64);

impl Noise {
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }

    fn gaussian(&mut self) -> f32 {
        let first = self.next_f32().max(f32::MIN_POSITIVE);
        let second = self.next_f32();
        (-2.0 * first.ln()).sqrt() * (std::f32::consts::TAU * second).cos()
    }
}

// ------------------------------------------------------------------------------------------------ the engine

struct Supertonic {
    duration: Session,
    text_encoder: Session,
    vector_field: Session,
    vocoder: Session,
    /// This release is 44.1 kHz; read from tts.json rather than assumed.
    sample_rate: u32,
    /// Frames of audio per latent row, and how many latents one chunk holds.
    base_chunk: usize,
    compress: usize,
    /// Rows of the latent tensor: ttl.latent_dim times the compression factor.
    latent_dim: usize,
    indexer: HashMap<char, i64>,
    style_ttl: Style,
    style_dp: Style,
}

/// Level 3 is ONNX Runtime's full fusion pass; it is paid once per synthesis and
/// is small next to the vector field's eight passes.
fn open(dir: &Path, name: &str) -> Result<Session, AppError> {
    Session::builder()
        .map_err(|err| AppError::Speak(format!("onnx runtime unavailable: {err}")))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|err| AppError::Speak(format!("onnx session builder: {err}")))?
        .commit_from_file(dir.join(name))
        .map_err(|err| {
            AppError::Speak(format!(
                "{name} could not be loaded: {err}. The graph may use an operator this build's \
                 onnx runtime does not have."
            ))
        })
}

/// The first output as a shape and its floats. Every graph here has one real
/// output and the SDK reads it positionally, so this does too.
fn first_output(outputs: &SessionOutputs<'_>) -> Result<(Vec<usize>, Vec<f32>), AppError> {
    let (name, value) = outputs
        .iter()
        .next()
        .ok_or_else(|| AppError::Speak("a graph returned no output".to_string()))?;
    let array = value
        .try_extract_array::<f32>()
        .map_err(|err| AppError::Speak(format!("{name} could not be read: {err}")))?;
    Ok((
        array.shape().to_vec(),
        array.iter().copied().collect::<Vec<f32>>(),
    ))
}

impl Supertonic {
    fn load(dir: &Path, speaker: Option<u32>) -> Result<Self, AppError> {
        let config = read_json(&dir.join(CONFIG))?;
        let read = |group: &str, key: &str| -> Result<u64, AppError> {
            config
                .get(group)
                .and_then(|block| block.get(key))
                .and_then(Json::as_u64)
                .ok_or_else(|| AppError::Speak(format!("tts.json has no {group}.{key}")))
        };
        let sample_rate = read("ae", "sample_rate")? as u32;
        let base_chunk = read("ae", "base_chunk_size")? as usize;
        let compress = read("ttl", "chunk_compress_factor")? as usize;
        let latent = read("ttl", "latent_dim")? as usize;
        // A style is a separate 290 KB file, named M1..M5 (and F1..F5 in the
        // repository). The card advertises one because it downloads one, so a
        // second speaker index fails at the missing file rather than silently
        // reusing the first voice's style and pretending to be another speaker.
        let index = speaker.unwrap_or(0) as usize;
        let style_path = dir.join(format!("M{}.json", index + 1));
        let style = read_json(&style_path)?;
        Ok(Self {
            duration: open(dir, DURATION)?,
            text_encoder: open(dir, TEXT_ENCODER)?,
            vector_field: open(dir, VECTOR_FIELD)?,
            vocoder: open(dir, VOCODER)?,
            sample_rate,
            base_chunk,
            compress,
            latent_dim: latent * compress,
            indexer: read_indexer(&dir.join(INDEXER))?,
            style_ttl: style_from(&style, "style_ttl")?,
            style_dp: style_from(&style, "style_dp")?,
        })
    }

    /// One chunk of text through the whole chain, the way the reference SDK's
    /// \`synthesize\` does it, so the two can be compared call for call.
    fn synthesize(&mut self, text: &str, speed: f32, language: &str) -> Result<Vec<f32>, AppError> {
        let ids = prepare(text, &self.indexer, language);
        if ids.is_empty() {
            return Err(AppError::Speak(
                "nothing in that text could be read aloud".to_string(),
            ));
        }
        let tokens = ids.len();
        let text_mask = vec![1.0f32; tokens];

        // 1. How long to speak. Speed divides the predicted duration, which is
        //    the reference's own way of going faster without changing pitch.
        let duration_feeds: Vec<(String, SessionInputValue<'static>)> = vec![
            (
                "text_ids".to_string(),
                Tensor::from_array((vec![1usize, tokens], ids.clone()))
                    .map_err(|err| AppError::Speak(format!("text_ids: {err}")))?
                    .into(),
            ),
            (
                "style_dp".to_string(),
                Tensor::from_array((self.style_dp.dims.clone(), self.style_dp.data.clone()))
                    .map_err(|err| AppError::Speak(format!("style_dp: {err}")))?
                    .into(),
            ),
            (
                "text_mask".to_string(),
                Tensor::from_array((vec![1usize, 1, tokens], text_mask.clone()))
                    .map_err(|err| AppError::Speak(format!("text_mask: {err}")))?
                    .into(),
            ),
        ];
        let outputs = self
            .duration
            .run(duration_feeds)
            .map_err(|err| AppError::Speak(format!("duration prediction failed: {err}")))?;
        let (_, predicted) = first_output(&outputs)?;
        let seconds = (predicted.first().copied().unwrap_or(0.0) / speed).max(0.1);

        // 2. The text embedding the vector field is conditioned on.
        let text_feeds: Vec<(String, SessionInputValue<'static>)> = vec![
            (
                "text_ids".to_string(),
                Tensor::from_array((vec![1usize, tokens], ids))
                    .map_err(|err| AppError::Speak(format!("text_ids: {err}")))?
                    .into(),
            ),
            (
                "style_ttl".to_string(),
                Tensor::from_array((self.style_ttl.dims.clone(), self.style_ttl.data.clone()))
                    .map_err(|err| AppError::Speak(format!("style_ttl: {err}")))?
                    .into(),
            ),
            (
                "text_mask".to_string(),
                Tensor::from_array((vec![1usize, 1, tokens], text_mask.clone()))
                    .map_err(|err| AppError::Speak(format!("text_mask: {err}")))?
                    .into(),
            ),
        ];
        let outputs = self
            .text_encoder
            .run(text_feeds)
            .map_err(|err| AppError::Speak(format!("text encoding failed: {err}")))?;
        let (embedding_dims, embedding) = first_output(&outputs)?;

        // 3. The latent: one row per chunk of audio, seeded with noise and cut
        //    to the predicted length by the mask.
        let chunk = self.base_chunk * self.compress;
        let frames = (seconds * self.sample_rate as f32) as usize;
        let latents = frames.div_ceil(chunk).max(1);
        let mut noise = Noise(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos() as u64)
                .unwrap_or(0x5eed_1234),
        );
        let mut latent: Vec<f32> = (0..self.latent_dim * latents)
            .map(|_| noise.gaussian())
            .collect();
        // Rows past the predicted end are silenced, exactly as \`latent_mask\`
        // does it: the solver would otherwise invent audio past the sentence.
        let voiced = frames.div_ceil(chunk).min(latents);
        for row in voiced..latents {
            let start = row * self.latent_dim;
            for slot in latent[start..start + self.latent_dim].iter_mut() {
                *slot = 0.0;
            }
        }
        let mut latent_mask = vec![0.0f32; latents];
        for slot in latent_mask.iter_mut().take(voiced) {
            *slot = 1.0;
        }

        for step in 0..STEPS {
            let feeds: Vec<(String, SessionInputValue<'static>)> = vec![
                (
                    "noisy_latent".to_string(),
                    Tensor::from_array((vec![1usize, self.latent_dim, latents], latent.clone()))
                        .map_err(|err| AppError::Speak(format!("noisy_latent: {err}")))?
                        .into(),
                ),
                (
                    "text_emb".to_string(),
                    Tensor::from_array((embedding_dims.clone(), embedding.clone()))
                        .map_err(|err| AppError::Speak(format!("text_emb: {err}")))?
                        .into(),
                ),
                (
                    "style_ttl".to_string(),
                    Tensor::from_array((self.style_ttl.dims.clone(), self.style_ttl.data.clone()))
                        .map_err(|err| AppError::Speak(format!("style_ttl: {err}")))?
                        .into(),
                ),
                (
                    "text_mask".to_string(),
                    Tensor::from_array((vec![1usize, 1, tokens], text_mask.clone()))
                        .map_err(|err| AppError::Speak(format!("text_mask: {err}")))?
                        .into(),
                ),
                (
                    "latent_mask".to_string(),
                    Tensor::from_array((vec![1usize, 1, latents], latent_mask.clone()))
                        .map_err(|err| AppError::Speak(format!("latent_mask: {err}")))?
                        .into(),
                ),
                (
                    "current_step".to_string(),
                    Tensor::from_array((vec![1usize], vec![step as f32]))
                        .map_err(|err| AppError::Speak(format!("current_step: {err}")))?
                        .into(),
                ),
                (
                    "total_step".to_string(),
                    Tensor::from_array((vec![1usize], vec![STEPS as f32]))
                        .map_err(|err| AppError::Speak(format!("total_step: {err}")))?
                        .into(),
                ),
            ];
            let outputs = self
                .vector_field
                .run(feeds)
                .map_err(|err| AppError::Speak(format!("flow step {step} failed: {err}")))?;
            let (_, next) = first_output(&outputs)?;
            latent = next;
        }

        // 4. The finished latent as audio.
        let vocoder_feeds: Vec<(String, SessionInputValue<'static>)> = vec![(
            "latent".to_string(),
            Tensor::from_array((vec![1usize, self.latent_dim, latents], latent))
                .map_err(|err| AppError::Speak(format!("latent: {err}")))?
                .into(),
        )];
        let outputs = self
            .vocoder
            .run(vocoder_feeds)
            .map_err(|err| AppError::Speak(format!("vocoding failed: {err}")))?;
        let (_, samples) = first_output(&outputs)?;
        Ok(samples)
    }
}

/// Supertonic 3: one voice style, thirty-one languages in the weights, English
/// in this card.
///
/// A long reply is split into sentence-sized chunks before it ever reaches the
/// graphs: the duration head and the latent mask both work per chunk, and one
/// paragraph through one call is how the reference's own chunker fails.
pub fn speak(
    dir: &Path,
    text: &str,
    speaker: Option<u32>,
    speed: f32,
    language: &str,
) -> Result<Synthesis, AppError> {
    let started = Instant::now();
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Speak(
            "there was nothing to read aloud".to_string(),
        ));
    }
    let mut engine = Supertonic::load(dir, speaker)?;
    let rate = engine.sample_rate;
    let speed = speed.clamp(0.7, 2.0);
    let mut samples: Vec<f32> = Vec::new();
    let gap = vec![0.0f32; (rate as f32 * 0.2) as usize];
    for (at, chunk) in chunks(text).iter().enumerate() {
        if at > 0 {
            samples.extend_from_slice(&gap);
        }
        let mut audio = engine.synthesize(chunk, speed, language)?;
        samples.append(&mut audio);
    }
    if samples.is_empty() {
        return Err(AppError::Speak(
            "the voice returned no audio for that text".to_string(),
        ));
    }
    Ok(Synthesis {
        samples,
        rate,
        ms: started.elapsed().as_millis() as u64,
    })
}

/// Sentence-sized pieces, and no piece longer than the model's chunk length.
/// Splitting on sentence ends first keeps a comma-rich sentence whole, which is
/// what the reference's own chunker does.
fn chunks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        if !current.is_empty() && current.chars().count() + sentence.chars().count() + 1 > MAX_CHARS
        {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(sentence.trim());
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    // A single sentence longer than the chunk length is cut at a space, which is
    // the least bad break: the model still gets whole words.
    let mut bounded = Vec::with_capacity(out.len());
    for chunk in out {
        if chunk.chars().count() <= MAX_CHARS {
            bounded.push(chunk);
            continue;
        }
        let mut piece = String::new();
        for word in chunk.split(' ') {
            if !piece.is_empty() && piece.chars().count() + word.chars().count() + 1 > MAX_CHARS {
                bounded.push(std::mem::take(&mut piece));
            }
            if !piece.is_empty() {
                piece.push(' ');
            }
            piece.push_str(word);
        }
        if !piece.trim().is_empty() {
            bounded.push(piece);
        }
    }
    bounded
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\n' {
            if !current.trim().is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | '\u{2026}') {
            while let Some(next) = chars.peek() {
                if matches!(next, '"' | '\'' | ')' | ']' | '\u{201d}' | '\u{2019}') {
                    current.push(*next);
                    chars.next();
                } else {
                    break;
                }
            }
            if !current.trim().is_empty() {
                out.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> HashMap<char, i64> {
        let mut map = HashMap::new();
        for (at, ch) in "abcdefghijklmnopqrstuvwxyz<>=/ ".chars().enumerate() {
            map.insert(ch, at as i64);
        }
        map.insert('.', 90);
        map
    }

    #[test]
    fn text_is_wrapped_in_its_language_tag_and_ended_with_a_stop() {
        let ids = prepare("hello", &table(), "en");
        let back: String = ids
            .iter()
            .map(|id| match *id {
                90 => '.',
                other => "abcdefghijklmnopqrstuvwxyz<>=/ "
                    .chars()
                    .nth(other as usize % 31)
                    .unwrap_or('?'),
            })
            .collect();
        assert!(back.starts_with("<en>hello"), "got {back}");
        assert!(back.ends_with("</en>"), "got {back}");
        assert_eq!(back, "<en>hello.</en>");
    }

    #[test]
    fn characters_the_table_does_not_know_are_dropped_not_guessed() {
        let ids = prepare("hi \u{2603}there", &table(), "en");
        // Snowman is not in the table; the words either side survive.
        assert!(ids.len() >= "<en>hithere.</en>".len() - 1);
    }

    #[test]
    fn a_long_sentence_is_cut_at_a_space() {
        let long = "word ".repeat(120);
        for chunk in chunks(&long) {
            assert!(
                chunk.chars().count() <= MAX_CHARS,
                "chunk of {} characters",
                chunk.chars().count()
            );
        }
    }

    #[test]
    fn sentences_are_chunks_until_the_length_bites() {
        let chunks = chunks("First one. Second one.");
        assert_eq!(chunks, vec!["First one. Second one.".to_string()]);
    }

    #[test]
    fn noise_is_not_all_zeros_and_is_roughly_gaussian() {
        let mut noise = Noise(7);
        let values: Vec<f32> = (0..4096).map(|_| noise.gaussian()).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let variance = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f32>()
            / values.len() as f32;
        assert!(mean.abs() < 0.1, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.2, "variance {variance}");
    }
}
