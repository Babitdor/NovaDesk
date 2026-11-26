//! Piper, run as a child process, one utterance at a time.
//!
//! There is no resident model: Piper loads one per run (~1.5 s of the measured
//! 2.4 s wall time for a 7 s utterance), which is why nothing here keeps an
//! idle timer — an idle process would hold hundreds of megabytes to save a load
//! the next reply pays anyway. What it does keep is a lock: two replies speaking
//! at once would each get half the CPU and finish at twice the wall time.
//!
//! Everything is asynchronous, because the wait is a child process rather than
//! our own arithmetic. The command layer calls this directly.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::speak::catalog::{TtsEngine, Voice};
use crate::speak::onnx;

/// Piper's own ceiling is not published; this is a transcript pasted into the
/// composer, not a reply read aloud, and the frontend splits long replies into
/// 320-character pieces before they ever arrive here.
pub const MAX_CHARS: usize = 4_000;
/// A wedged child must not hold the turn open forever. The 120 s deadline is
/// generous for 4,000 characters at the slowest speed.
const TIMEOUT: Duration = Duration::from_secs(120);
/// Below and above this the request is a mistake rather than a preference; the
/// page clamps too, but a hand-written IPC call would otherwise ask Piper for a
/// whisper or a drawl.
const MIN_SPEED: f32 = 0.5;
const MAX_SPEED: f32 = 2.0;
/// Where the finished wave lands before it is read back and deleted. One file,
/// because the lock above means one synthesis at a time.
pub const SCRATCH: &str = "synthesize.wav";

#[derive(Debug, Clone)]
pub struct Synthesis {
    /// Mono, at `rate`, in -1..1.
    pub samples: Vec<f32>,
    pub rate: u32,
    pub ms: u64,
}

/// The process-wide holder. No weights, only the lock.
#[derive(Default)]
pub struct SpeakEngine {
    running: Mutex<()>,
}

impl SpeakEngine {
    /// Speak one piece of text with one voice.
    ///
    /// Refuses, rather than returning silence, when the voice cannot run here,
    /// when its files are not installed, when there is nothing to say, or when
    /// the reply is longer than one request.
    pub async fn synthesize(
        &self,
        voice: &'static Voice,
        dir: &Path,
        exe: &Path,
        text: &str,
        speaker: Option<u32>,
        speed: f32,
    ) -> Result<Synthesis, AppError> {
        if !voice.supported {
            return Err(AppError::Speak(
                voice
                    .unsupported_reason
                    .unwrap_or("this voice cannot run on this build")
                    .to_string(),
            ));
        }
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
        let missing: Vec<&str> = voice
            .required
            .iter()
            .copied()
            .filter(|name| !dir.join(name).is_file())
            .collect();
        if !missing.is_empty() {
            return Err(AppError::Speak(format!(
                "{} is not installed; download it in Settings, Voice",
                voice.name
            )));
        }
        // One utterance at a time, whichever engine speaks it: a child process
        // and an ONNX session both want the whole CPU, so two replies at once
        // would each finish at twice the wall time.
        let _guard = self.running.lock().await;

        match voice.engine {
            TtsEngine::Piper => speak_piper(voice, dir, exe, text, speaker, speed).await,
            TtsEngine::Kale => {
                // ONNX inference is blocking arithmetic rather than a child
                // process, so it goes to the blocking pool: the async runtime
                // must not be held while this task is busy on its own cores.
                let dir = dir.to_path_buf();
                let text = text.to_string();
                tokio::task::spawn_blocking(move || onnx::kale(&dir, &text, speaker, speed))
                    .await
                    .map_err(|err| {
                        AppError::Speak(format!("{} stopped unexpectedly: {err}", voice.name))
                    })?
            }
            TtsEngine::Kokoro => {
                // Kokoro reads the shared phoneme front end, installed beside
                // the voices, so its own folder holds only the graph and style.
                let dir = dir.to_path_buf();
                let text = text.to_string();
                tokio::task::spawn_blocking(move || crate::speak::kokoro::speak(&dir, &text, speed))
                    .await
                    .map_err(|err| {
                        AppError::Speak(format!("{} stopped unexpectedly: {err}", voice.name))
                    })?
            }
            TtsEngine::Supertonic => {
                // Four graphs and eight flow steps: also blocking, also behind
                // the same lock, so two replies never halve each other's cores.
                // The language comes from the voice's own code, so a card for
                // another language needs no change here.
                let dir = dir.to_path_buf();
                let text = text.to_string();
                let language = voice
                    .language
                    .split('_')
                    .next()
                    .unwrap_or(crate::speak::supertonic::DEFAULT_LANGUAGE)
                    .to_string();
                tokio::task::spawn_blocking(move || {
                    crate::speak::supertonic::speak(&dir, &text, speaker, speed, &language)
                })
                .await
                .map_err(|err| {
                    AppError::Speak(format!("{} stopped unexpectedly: {err}", voice.name))
                })?
            }
        }
    }
}

/// Piper, one child process per utterance.
async fn speak_piper(
    voice: &'static Voice,
    dir: &Path,
    exe: &Path,
    text: &str,
    speaker: Option<u32>,
    speed: f32,
) -> Result<Synthesis, AppError> {
    if !exe.is_file() {
        return Err(AppError::Speak(
            "Piper is not installed; download a voice in Settings, Voice and it arrives with it"
                .to_string(),
        ));
    }

    let start = Instant::now();
    let scratch = scratch_path(dir);
    // By suffix rather than by position: the catalogue lists two files, and
    // which one is the weights is Piper's convention, not an index.
    let weights = voice
        .required
        .iter()
        .find(|name| name.ends_with(".onnx"))
        .ok_or_else(|| AppError::Speak(format!("{} has no weights to load", voice.name)))?;
    let description = voice
        .required
        .iter()
        .find(|name| name.ends_with(".onnx.json"))
        .ok_or_else(|| {
            AppError::Speak(format!("{} has no voice description to load", voice.name))
        })?;
    let model = dir.join(weights);
    let config = dir.join(description);
    let speed = speed.clamp(MIN_SPEED, MAX_SPEED);
    // Piper's length scale is the opposite of a speed: bigger is slower.
    let length_scale = 1.0 / speed;

    let mut command = Command::new(exe);
    command
        .arg("-q")
        .arg("--model")
        .arg(&model)
        .arg("--config")
        .arg(&config)
        .arg("--output_file")
        .arg(&scratch)
        .arg("--length_scale")
        .arg(format!("{length_scale:.3}"));
    if voice.speakers > 1 {
        // Only multi-speaker weights understand -s; passing it to the rest
        // is an error, not a default.
        let speaker = speaker.unwrap_or(0).min(voice.speakers - 1);
        command.arg("--speaker").arg(speaker.to_string());
    }
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let run = async {
        let mut child = command
            .spawn()
            .map_err(|err| AppError::Speak(format!("Piper could not be started ({err})")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes()).await.map_err(|err| {
                AppError::Speak(format!("the reply could not be sent to Piper ({err})"))
            })?;
            // Closing stdin is what tells Piper the paragraph is finished.
            drop(stdin);
        }
        child
            .wait_with_output()
            .await
            .map_err(|err| AppError::Speak(format!("{} could not speak: {err}", voice.name)))
    };

    let output = tokio::time::timeout(TIMEOUT, run)
        .await
        .map_err(|_| {
            AppError::Speak(format!(
                "{} took longer than {} seconds to speak, so it was stopped",
                voice.name,
                TIMEOUT.as_secs()
            ))
        })?
        .map_err(|err| AppError::Speak(format!("{} could not speak: {err}", voice.name)))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim().lines().next().unwrap_or("");
        let _ = std::fs::remove_file(&scratch);
        return Err(AppError::Speak(if detail.is_empty() {
            format!(
                "{} could not speak (Piper exited with {})",
                voice.name, output.status
            )
        } else {
            format!("{} could not speak: {detail}", voice.name)
        }));
    }

    let raw = tokio::fs::read(&scratch)
        .await
        .map_err(|err| AppError::Speak(format!("{} produced no audio ({err})", voice.name)));
    // The scratch file is ours and worthless once read; a leftover would be
    // played back by nothing and confuse the next run's error message.
    let _ = std::fs::remove_file(&scratch);
    let raw = raw?;
    let (samples, rate) = parse_wav(&raw).map_err(|err| {
        AppError::Speak(format!(
            "{} produced audio Nova could not read: {err}",
            voice.name
        ))
    })?;

    Ok(Synthesis {
        samples,
        rate,
        ms: start.elapsed().as_millis() as u64,
    })
}

fn scratch_path(dir: &Path) -> PathBuf {
    dir.parent().unwrap_or(dir).join(SCRATCH)
}

/// Read a RIFF/WAVE file into mono f32.
///
/// The parser is ours because the alternative was a crates.io WAV reader for
/// one format that Piper always writes the same way: fmt then data, PCM16,
/// little endian. It still checks what it reads — a header that lies about its
/// length is how a killed process looks, and playing half a sentence as though
/// it were the whole reply is worse than an error.
pub fn parse_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".to_string());
    }
    let mut offset = 12;
    let mut format = 0u16;
    let mut channels = 0u16;
    let mut rate = 0u32;
    let mut bits = 0u16;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        match id {
            b"fmt " => {
                if size < 16 || body + 16 > bytes.len() {
                    return Err("the format description is truncated".to_string());
                }
                format = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
                channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
                rate = u32::from_le_bytes([
                    bytes[body + 4],
                    bytes[body + 5],
                    bytes[body + 6],
                    bytes[body + 7],
                ]);
                bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            }
            b"data" => {
                if format != 1 || bits != 16 {
                    return Err(format!(
                        "unsupported encoding (format {format}, {bits}-bit); Nova reads 16-bit PCM"
                    ));
                }
                if channels == 0 || rate == 0 {
                    return Err("the file names no channels or no sample rate".to_string());
                }
                let available = bytes.len().saturating_sub(body);
                if size > available {
                    return Err(format!(
                        "the audio data stops at {available} of {size} bytes"
                    ));
                }
                let frame = 2 * channels as usize;
                let frames = size / frame;
                if frames == 0 {
                    return Err("the audio data is empty".to_string());
                }
                let mut samples = Vec::with_capacity(frames);
                for index in 0..frames {
                    let base = body + index * frame;
                    let mut sum = 0f32;
                    for channel in 0..channels as usize {
                        let at = base + channel * 2;
                        sum += i16::from_le_bytes([bytes[at], bytes[at + 1]]) as f32 / 32_768.0;
                    }
                    samples.push(sum / channels as f32);
                }
                return Ok((samples, rate));
            }
            _ => {}
        }
        // Chunks are word aligned; an odd size is followed by one pad byte.
        offset = body + size + (size & 1);
    }
    Err("the file has no audio data".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real 44-byte header, the shape Piper writes: fmt then data, mono PCM16.
    fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
        let data: Vec<u8> = samples
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn reads_a_real_header() {
        let (samples, rate) =
            parse_wav(&wav(&[0, 16_384, -16_384, 32_767], 22_050)).expect("a well-formed file");
        assert_eq!(rate, 22_050);
        assert_eq!(samples.len(), 4);
        assert_eq!(samples[0], 0.0);
        assert!((samples[1] - 0.5).abs() < 0.001);
        assert!((samples[2] + 0.5).abs() < 0.001);
        assert!(samples[3] > 0.999);
    }

    #[test]
    fn refuses_a_file_that_stops_early() {
        let mut cut = wav(&[1, 2, 3, 4, 5, 6], 16_000);
        cut.truncate(cut.len() - 3);
        let err = parse_wav(&cut).expect_err("a truncated file is not a reply");
        assert!(err.contains("stops at"), "unexpected message: {err}");
    }

    #[test]
    fn refuses_something_that_is_not_a_wave_file() {
        assert!(parse_wav(b"RIFF").is_err());
        assert!(parse_wav(b"").is_err());
        assert!(parse_wav(b"RIFF0000WAVEfmt ").is_err());
    }

    #[test]
    fn refuses_a_partial_frame() {
        // A data chunk whose size is not a whole number of 16-bit frames.
        let mut file = wav(&[1, 2, 3, 4], 16_000);
        let last = file.len();
        file[last - 1] = 0x7f;
        file.truncate(last - 1);
        let err = parse_wav(&file).expect_err("a partial frame is a bad file");
        assert!(err.contains("stops at"), "unexpected message: {err}");
    }

    #[test]
    fn the_scratch_file_sits_beside_the_voice_folder() {
        let path = scratch_path(Path::new("/speak/en_US-amy-low"));
        assert_eq!(path, Path::new("/speak/synthesize.wav"));
    }
}
