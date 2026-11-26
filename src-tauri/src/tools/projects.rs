//! The project list: every workspace Nova has opened.
//!
//! Nova keeps the open workspace in memory, so a fresh launch knows one folder
//! and a tool asked "what projects are there" would otherwise answer with a
//! single entry. This file is written by set_workspace and read by the
//! list_projects tool, which is why it lives in the app data folder rather than
//! in any workspace: opening folder B must not look like folder A remembering
//! nothing.
//!
//! Written atomically: a half-written list would lose every earlier project the
//! first time the app was killed mid-save.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::AppError;

pub const FILE: &str = "projects.json";

/// At 50 folders the list stops being a history and starts being a liability:
/// the whole file is rewritten on every open.
const MAX_PROJECTS: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Stable across launches, derived from the path, so a tool can name a
    /// project without inventing an index that shifts on the next open.
    pub id: String,
    pub name: String,
    pub path: String,
    pub opened_at: u64,
}

fn file(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// A stable id from the canonical path. Not a security boundary, just a name
/// that does not change when the recency order does.
fn id_for(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn load(data_dir: &Path) -> Vec<Project> {
    let raw = match std::fs::read(file(data_dir)) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    serde_json::from_slice::<Vec<Project>>(&raw).unwrap_or_default()
}

/// Records a workspace as the newest project. Errors are the caller's to
/// report: a failed save must not stop a folder from opening.
pub fn register(data_dir: &Path, root: &Path) -> Result<(), AppError> {
    let mut projects = load(data_dir);
    let path = root.display().to_string();
    projects.retain(|project| project.path != path);
    projects.insert(
        0,
        Project {
            id: id_for(root),
            name: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
            path,
            opened_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or_default(),
        },
    );
    projects.truncate(MAX_PROJECTS);
    std::fs::create_dir_all(data_dir).map_err(|err| AppError::from_io(err, data_dir))?;
    let target = file(data_dir);
    let temp = target.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(&projects)
        .map_err(|err| AppError::Io(format!("could not serialise the project list: {err}")))?;
    std::fs::write(&temp, body).map_err(|err| AppError::from_io(err, &temp))?;
    std::fs::rename(&temp, &target).map_err(|err| AppError::from_io(err, &target))
}

/// The tool's answer. A missing data folder is not an error: it means nothing
/// has been opened yet.
pub fn list(data_dir: Option<&Path>) -> Value {
    let projects = data_dir.map(load).unwrap_or_default();
    json!({ "projects": projects, "count": projects.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    #[test]
    fn the_newest_project_is_first_and_reopening_moves_it_up() {
        let data = TempDir::new("projects-data");
        let first = TempDir::new("projects-a");
        let second = TempDir::new("projects-b");
        register(data.path(), first.path()).unwrap();
        register(data.path(), second.path()).unwrap();
        let projects = load(data.path());
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].path, second.path().display().to_string());

        register(data.path(), first.path()).unwrap();
        let projects = load(data.path());
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].path, first.path().display().to_string());
        assert_eq!(projects[0].id, id_for(first.path()));
    }

    #[test]
    fn a_corrupt_file_reads_as_no_projects() {
        let data = TempDir::new("projects-corrupt");
        std::fs::write(file(data.path()), b"not json").unwrap();
        assert!(load(data.path()).is_empty());
        assert_eq!(list(Some(data.path()))["count"], 0);
    }
}
