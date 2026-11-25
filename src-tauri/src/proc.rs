use std::process::Stdio;

use tauri::ipc::Channel;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::events::{AgentEvent, OutputStream};
use crate::workspace::WorkspaceGuard;

/// Hard cap per stream so a chatty child cannot exhaust memory or the IPC channel.
const MAX_STREAM_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Copy)]
pub struct ProcOutcome {
    pub exit_code: Option<i32>,
    pub truncated: bool,
}

impl ProcOutcome {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

pub struct ProcessRunner;

impl ProcessRunner {
    /// Spawns `program` with `args` directly — never through a shell — in the
    /// workspace root, streaming both pipes and killing the child on cancel.
    pub async fn run(
        guard: &WorkspaceGuard,
        program: &str,
        args: &[String],
        sink: &Channel<AgentEvent>,
        cancel: &CancellationToken,
    ) -> Result<ProcOutcome, AppError> {
        if program.trim().is_empty() {
            return Err(AppError::Spawn("empty program name".into()));
        }

        let mut child = Command::new(program)
            .args(args)
            .current_dir(guard.root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Safety net: if this future is dropped, the child dies with it.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| AppError::Spawn(format!("{program}: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Spawn("stdout was not piped".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Spawn("stderr was not piped".into()))?;

        let out_task = tokio::spawn(pump(stdout, OutputStream::Stdout, sink.clone()));
        let err_task = tokio::spawn(pump(stderr, OutputStream::Stderr, sink.clone()));

        // The handles move into one arm only; the cancel arm must not touch them.
        let pumped = tokio::select! {
            _ = cancel.cancelled() => None,
            joined = async { tokio::join!(out_task, err_task) } => Some(joined),
        };

        let Some(pumped) = pumped else {
            let _ = child.start_kill();
            // Reaping here also closes the pipes, so the detached pumps reach EOF.
            let _ = child.wait().await;
            return Err(AppError::Cancelled);
        };

        let (out_truncated, err_truncated) = match pumped {
            (Ok(a), Ok(b)) => (a, b),
            _ => return Err(AppError::Spawn("output pump task failed".into())),
        };

        let status = child
            .wait()
            .await
            .map_err(|e| AppError::Io(e.to_string()))?;

        Ok(ProcOutcome {
            exit_code: status.code(),
            truncated: out_truncated || err_truncated,
        })
    }
}

/// Streams one pipe into `sink`. Returns whether the byte cap was hit.
///
/// shortcut: chunks are decoded independently, so a multi-byte character split
/// across a read boundary becomes U+FFFD. Upgrade to a carry-buffer decoder if
/// non-ASCII process output becomes common.
async fn pump<R>(mut reader: R, stream: OutputStream, sink: Channel<AgentEvent>) -> bool
where
    R: AsyncRead + Unpin,
{
    let mut buf = vec![0u8; 8192];
    let mut emitted = 0usize;

    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };

        // Past the cap, keep draining so a chatty child never blocks on a full pipe.
        if emitted >= MAX_STREAM_BYTES {
            continue;
        }
        let n = n.min(MAX_STREAM_BYTES - emitted);
        emitted += n;

        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
        let _ = sink.send(AgentEvent::ProcessOutput { stream, chunk });
    }

    emitted >= MAX_STREAM_BYTES
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::events::testutil::collect_events;
    use crate::testtmp::TempDir;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn streams_child_output_into_events() {
        let tmp = TempDir::new("proc-stream");
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        let (sink, seen) = collect_events();

        let outcome = ProcessRunner::run(
            &guard,
            "cmd",
            &args(&["/C", "echo", "hello-from-child"]),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(outcome.exit_code, Some(0));
        assert!(!outcome.truncated);

        let text: String = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ProcessOutput { chunk, .. } => Some(chunk.clone()),
                _ => None,
            })
            .collect();
        assert!(text.contains("hello-from-child"), "got {text:?}");
    }

    #[tokio::test]
    async fn cancel_kills_a_long_running_child() {
        let tmp = TempDir::new("proc-cancel");
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        let (sink, _seen) = collect_events();

        let cancel = CancellationToken::new();
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
        });

        // ping -n 20 would take ~19s, so finishing quickly proves the kill landed.
        let started = Instant::now();
        let err = ProcessRunner::run(
            &guard,
            "cmd",
            &args(&["/C", "ping", "-n", "20", "127.0.0.1"]),
            &sink,
            &cancel,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Cancelled), "got {err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cancel took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn reports_a_missing_program_instead_of_panicking() {
        let tmp = TempDir::new("proc-missing");
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        let (sink, _seen) = collect_events();

        let err = ProcessRunner::run(
            &guard,
            "nova-definitely-not-a-real-program",
            &[],
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Spawn(_)), "got {err:?}");
    }
}
