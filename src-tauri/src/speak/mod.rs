//! Text-to-speech: what Nova can read aloud, and how it does it.
//!
//! Two destinations with nothing in common beyond this module: a Piper binary
//! downloaded next to its voices (engine.rs, one process per utterance), and a
//! hosted speech service (service.rs, one HTTPS POST per utterance). The
//! catalogue describes both, because the page has to show a voice's size before
//! the user has chosen anything.

pub mod catalog;
pub mod engine;
pub mod frontend;
pub mod kokoro;
pub mod onnx;
pub mod service;
pub mod supertonic;
