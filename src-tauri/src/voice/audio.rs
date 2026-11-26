//! Sample preparation between the webview and an engine.
//!
//! The page records through MediaRecorder, decodes the blob with an
//! AudioContext and ships the raw decoded samples here: interleaved, at the
//! device rate, as little-endian f32. Every engine wants 16 kHz mono, so the
//! conversion happens once, in Rust, where it can be tested - the browser layer
//! stays a transport.

use crate::error::AppError;

/// What every engine in transcribe-rs expects.
pub const TARGET_RATE: u32 = 16_000;

/// Longest accepted dictation. The samples cross the IPC boundary as base64, so
/// an hour-long capture would cost more to move than to transcribe; the page
/// stops recording at 60 s and this is the hard backstop.
pub const MAX_SECONDS: u32 = 300;

/// Downmix to mono and resample to 16 kHz.
///
/// A device rate above the target is averaged per output sample (a box filter),
/// which keeps the amplitude of a steady signal instead of aliasing it; a rate
/// below the target is interpolated linearly. Both directions keep the duration
/// within one input sample of the original.
pub fn to_model_input(interleaved: &[f32], channels: u16, rate: u32) -> Result<Vec<f32>, AppError> {
    if channels == 0 || rate == 0 {
        return Err(AppError::Voice(
            "the recording reported no channels or no sample rate".to_string(),
        ));
    }
    if interleaved.len() % channels as usize != 0 {
        return Err(AppError::Voice(format!(
            "the recording has {} samples, which is not a whole number of {channels}-channel frames",
            interleaved.len()
        )));
    }
    let frames = interleaved.len() / channels as usize;
    if frames == 0 {
        return Err(AppError::Voice("the recording is empty".to_string()));
    }
    let seconds = frames as f64 / rate as f64;
    if seconds > MAX_SECONDS as f64 {
        return Err(AppError::Voice(format!(
            "the recording is {seconds:.0} s long; the limit is {MAX_SECONDS} s"
        )));
    }

    let mono = downmix(interleaved, channels as usize);
    Ok(resample(&mono, rate, TARGET_RATE))
}

/// Average every channel into one. Averaging (rather than picking a channel)
/// keeps a stereo recording centred, which is what the mic actually captured.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resample mono samples. Length is floor(len * to / from), so 48 kHz to
/// 16 kHz is exactly one third.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let from = from as u64;
    let to = to as u64;
    let len = input.len() as u64;
    let out_len = (len * to / from) as usize;
    let mut output = Vec::with_capacity(out_len);

    if from > to {
        // Downsampling: each output sample averages the input span it covers.
        for index in 0..out_len as u64 {
            let start = (index * from / to) as usize;
            let end = (((index + 1) * from / to) as usize)
                .max(start + 1)
                .min(input.len());
            let span = &input[start..end];
            output.push(span.iter().sum::<f32>() / span.len() as f32);
        }
    } else {
        // Upsampling: linear interpolation, holding the last sample at the end.
        let step = from as f64 / to as f64;
        for index in 0..out_len {
            let position = index as f64 * step;
            let left = position.floor() as usize;
            let fraction = (position - left as f64) as f32;
            let a = input[left.min(input.len() - 1)];
            let b = input[(left + 1).min(input.len() - 1)];
            output.push(a + (b - a) * fraction);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_frames() {
        let error = to_model_input(&[0.0, 0.0, 0.0], 2, 48_000).unwrap_err();
        assert!(error.to_string().contains("whole number"), "{error}");
    }

    #[test]
    fn rejects_empty_recordings() {
        assert!(to_model_input(&[], 1, 48_000).is_err());
    }

    #[test]
    fn keeps_a_16k_signal_untouched() {
        let samples = vec![0.25, -0.5, 0.75];
        assert_eq!(to_model_input(&samples, 1, TARGET_RATE).unwrap(), samples);
    }
}
