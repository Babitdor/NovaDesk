use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Errors that cross the IPC boundary.
///
/// Serialized as `{ kind, message }` so the UI can branch on `kind` instead of
/// parsing prose; `AppError` is the only error type any command returns.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("no workspace is open")]
    NoWorkspace,
    #[error("Nova requires approval for this workspace.")]
    WorkspaceNotApproved,
    #[error("path escapes the workspace: {0}")]
    PathEscape(String),
    #[error("path not found: {0}")]
    NotFound(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("not a file: {0}")]
    NotAFile(String),
    #[error("file is too large to display: {0}")]
    TooLarge(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("could not start process: {0}")]
    Spawn(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Agent(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::NoWorkspace => "noWorkspace",
            AppError::WorkspaceNotApproved => "workspaceNotApproved",
            AppError::PathEscape(_) => "pathEscape",
            AppError::NotFound(_) => "notFound",
            AppError::NotADirectory(_) => "notADirectory",
            AppError::NotAFile(_) => "notAFile",
            AppError::TooLarge(_) => "tooLarge",
            AppError::Io(_) => "io",
            AppError::Spawn(_) => "spawn",
            AppError::Cancelled => "cancelled",
            AppError::Agent(_) => "agent",
        }
    }

    /// Maps an io::Error to the variant the UI can act on: ENOENT means the user
    /// pointed at something that no longer exists, everything else is an I/O fault.
    pub fn from_io(err: std::io::Error, path: &std::path::Path) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound(path.display().to_string()),
            _ => AppError::Io(format!("{}: {err}", path.display())),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}
