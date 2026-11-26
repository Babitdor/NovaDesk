//! Hosted speech-to-text: one recording sent to a service instead of a model
//! running on this computer.
//!
//! The on-device path lives in `crate::voice`; this is the second destination a
//! dictation can have, and it shares nothing with it but the samples it is
//! handed.

pub mod service;
