//! Shared HTTP plumbing for the two things Nova asks a server for a body of
//! bytes: GitHub's REST API and the speech services.
//!
//! Everything goes through the system curl, for the same reason the theme
//! installer and the model downloads do: it is already on every host Nova
//! supports, it speaks TLS and redirects, and it never links an HTTP stack into
//! this executable.
//!
//! Two rules live here rather than in a caller:
//!
//! - A credential travels in a header **file**, never in argv. A command line is
//!   readable by every process on the machine and ends up in a crash report, so
//!   `-H @file` is the only shape allowed. The file is 0600 where the OS has
//!   modes and is deleted the moment the transfer finishes.
//! - The status code is read from `--write-out` into a separate stream, not
//!   appended to the body: an mp3 response will happily contain a newline and
//!   three digits, and splitting the body on the last newline would then turn a
//!   download into a parse error.

use std::path::{Path, PathBuf};

use crate::commands::settings::{capture, system_command};
use crate::error::AppError;

/// A file that deletes itself when the transfer that needed it is over.
pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Write one file in the temp folder, readable only by this user.
fn temp_file(prefix: &str, suffix: &str, body: &[u8]) -> Result<TempFile, AppError> {
    let path = std::env::temp_dir().join(format!("nova-{prefix}-{}{suffix}", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|err| AppError::from_io(err, &path))?;
    use std::io::Write;
    file.write_all(body)
        .map_err(|err| AppError::from_io(err, &path))?;
    Ok(TempFile { path })
}

/// The Authorization header, in a file rather than in argv: a command line is
/// visible to every process on the machine, and to the crash reporter.
pub fn write_header_file(token: &str) -> Result<TempFile, AppError> {
    temp_file(
        "github",
        ".hdr",
        format!("Authorization: Bearer {token}\r\n").as_bytes(),
    )
}

/// A request body, in a file for the same reason as the header and for one
/// more: a reply read aloud is thousands of characters, and Windows would
/// re-quote every one of them on a command line.
fn write_body_file(body: &str) -> Result<TempFile, AppError> {
    temp_file("body", ".json", body.as_bytes())
}

/// Refuses anything that is not a plain https URL.
///
/// `--proto =https` already stops curl, but a URL carrying credentials
/// (`https://user:pass@host/`) would still be sent, and one built by a
/// profile is user input. Both are checked before curl runs, and the redirect
/// chain cannot leave https either.
pub fn https_url(url: &str) -> Result<(), AppError> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| AppError::Speak("the address must start with https://".to_string()))?;
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        return Err(AppError::Speak("the address has no host".to_string()));
    }
    if authority.contains('@') {
        return Err(AppError::Speak(
            "the address must not carry a username or password".to_string(),
        ));
    }
    Ok(())
}

/// One POST with a JSON body, returning the body bytes and the status code.
///
/// `header` is the complete header line (`Authorization: Bearer …` or
/// `xi-api-key: …`), because the speech providers do not agree on its name.
/// curl runs without `--fail` so an error keeps its status: the caller turns a
/// 401 into "the key was rejected" rather than "the command failed".
pub async fn post_for_bytes(
    url: &str,
    header: Option<&str>,
    json_body: &str,
    limit: u64,
) -> Result<(Vec<u8>, u16), AppError> {
    https_url(url)?;

    let header_file = match header {
        Some(line) => Some(temp_file(
            "speech",
            ".hdr",
            format!("{line}\r\n").as_bytes(),
        )?),
        None => None,
    };
    let body_file = write_body_file(json_body)?;
    let out = temp_file("speech", ".body", b"")?;

    let mut command = system_command("curl");
    transfer_flags(&mut command, limit);
    command.args([
        "-H",
        "Content-Type: application/json",
        "-X",
        "POST",
        "--data-binary",
    ]);
    command.arg(format!("@{}", body_file.path().display()));
    if let Some(file) = &header_file {
        command.arg("-H").arg(format!("@{}", file.path().display()));
    }
    deliver(command, &out, url).await
}

/// How long one transfer may take, end to end. A dictation upload can be 9.6 MB
/// — 300 s of 16 kHz mono WAV — which is over a minute on a slow uplink; the
/// deadline only has to stop a hung connection, not a large but moving one.
const MAX_TRANSFER_SECONDS: u64 = 180;

/// The flags every transfer here shares: https only, quiet, bounded, and the
/// status code on its own stream.
fn transfer_flags(command: &mut tokio::process::Command, limit: u64) {
    command.args([
        "-q",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-redirs",
        "3",
        "--max-time",
        &MAX_TRANSFER_SECONDS.to_string(),
        "--max-filesize",
        &limit.to_string(),
        "--user-agent",
        "NovaDesktop",
    ]);
}

/// Runs the prepared request, then reads the body file and the status curl
/// wrote out.
async fn deliver(
    mut command: tokio::process::Command,
    out: &TempFile,
    url: &str,
) -> Result<(Vec<u8>, u16), AppError> {
    command.arg("--output").arg(out.path());
    command.args(["--write-out", "%{http_code}", "--"]).arg(url);

    // One sentence for both callers: this runs for speech and for dictation,
    // and curl's own exit reason is not something the page can act on.
    let code = capture(command, 16).await.map_err(|_| {
        AppError::Speak("the service could not be reached; check the connection".to_string())
    })?;
    let status = String::from_utf8_lossy(&code)
        .trim()
        .parse::<u16>()
        .unwrap_or(0);
    let bytes = tokio::fs::read(out.path())
        .await
        .map_err(|err| AppError::from_io(err, out.path()))?;
    Ok((bytes, status))
}

/// One POST sent as multipart/form-data, returning the body bytes and the
/// status code.
///
/// This is the shape every hosted transcriber takes: the recording as one file
/// field, then the model and the language as text fields. A form body cannot go
/// in a file the way a JSON body can, so each field is one argv element with no
/// shell in between, and `--form-string` keeps curl from reading a value that
/// begins with an at sign as a filename. The upload goes in a temp file for the
/// usual reason: a microphone recording is megabytes.
pub async fn post_form_for_bytes(
    url: &str,
    header: Option<&str>,
    upload: (&str, &[u8], &str),
    fields: &[(&str, &str)],
    limit: u64,
) -> Result<(Vec<u8>, u16), AppError> {
    https_url(url)?;
    let (field, bytes, mime) = upload;

    let header_file = match header {
        Some(line) => Some(temp_file(
            "transcribe",
            ".hdr",
            format!("{line}\r\n").as_bytes(),
        )?),
        None => None,
    };
    let audio = temp_file("transcribe", ".wav", bytes)?;
    let out = temp_file("transcribe", ".json", b"")?;

    let mut command = system_command("curl");
    transfer_flags(&mut command, limit);
    // The path is quoted inside the form argument: curl splits an unquoted
    // at-sign path followed by a semicolon and a type on `;` and `,`, and a
    // Windows profile folder holds both often enough to matter.
    command.arg("-F").arg(format!(
        "{field}=@\"{}\";type={mime}",
        audio.path().display()
    ));
    for (name, value) in fields {
        command.arg("--form-string").arg(format!("{name}={value}"));
    }
    if let Some(file) = &header_file {
        command.arg("-H").arg(format!("@{}", file.path().display()));
    }
    deliver(command, &out, url).await
}
