use std::path::{Path, PathBuf};

use crate::error::AppError;

/// The one place a path becomes trustworthy.
///
/// Canonicalizing both the root (once, here) and the candidate means `..`,
/// absolute paths and symlinks are all resolved to the same lexical form before
/// the containment check, which is why a plain `starts_with` is sound.
///
/// shortcut: check-then-use has a TOCTOU window; acceptable for a local
/// single-user app because the same user already owns the files. Upgrade with
/// handle-based APIs if this ever serves untrusted input.
#[derive(Debug, Clone)]
pub struct WorkspaceGuard {
    root: PathBuf,
}

impl WorkspaceGuard {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, AppError> {
        let requested = root.as_ref();
        let root = std::fs::canonicalize(requested).map_err(|e| AppError::from_io(e, requested))?;
        if !root.is_dir() {
            return Err(AppError::NotADirectory(requested.display().to_string()));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves an existing path (file or directory) inside the workspace.
    pub fn resolve(&self, requested: &str) -> Result<PathBuf, AppError> {
        let candidate = self.root.join(requested);
        let resolved =
            std::fs::canonicalize(&candidate).map_err(|e| AppError::from_io(e, &candidate))?;
        self.contain(resolved, requested)
    }

    /// Resolves a path that may not exist yet: the parent must be inside the
    /// workspace, the final component is taken verbatim.
    pub fn resolve_for_write(&self, requested: &str) -> Result<PathBuf, AppError> {
        let candidate = self.root.join(requested);
        let parent = candidate
            .parent()
            .ok_or_else(|| AppError::PathEscape(requested.to_string()))?;
        let name = candidate
            .file_name()
            .ok_or_else(|| AppError::PathEscape(requested.to_string()))?;
        let parent = std::fs::canonicalize(parent)
            .map_err(|e| AppError::from_io(e, Path::new(requested)))?;
        let parent = self.contain(parent, requested)?;
        Ok(parent.join(name))
    }

    /// Workspace-relative, forward-slashed form — the only path shape the UI sees.
    pub fn display_relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn contain(&self, resolved: PathBuf, requested: &str) -> Result<PathBuf, AppError> {
        if resolved.starts_with(&self.root) {
            Ok(resolved)
        } else {
            Err(AppError::PathEscape(requested.to_string()))
        }
    }
}

/// The label a picked attachment travels under: workspace-relative when a folder
/// is open, otherwise the path itself with forward slashes. A harness reads
/// either, and the UI never has to print a Windows separator.
pub(crate) fn attachment_label(guard: Option<&WorkspaceGuard>, path: &Path) -> String {
    let Some(guard) = guard else {
        return path.to_string_lossy().replace('\\', "/");
    };
    // A picker hands back a plain host path while the guard holds a canonical one
    // (verbatim, `\\?\C:\...` on Windows), so the forms have to be compared like
    // with like — otherwise nothing inside the workspace ever looks inside it.
    // A path outside keeps its own readable form rather than the verbatim one.
    match std::fs::canonicalize(path) {
        Ok(real) if real.starts_with(guard.root()) => guard.display_relative(&real),
        _ => path.to_string_lossy().replace('\\', "/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    #[test]
    fn attachment_labels_are_relative_inside_and_absolute_outside() {
        let tmp = TempDir::new("attach-label");
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/a.ts"), "x").unwrap();
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();

        let inside = tmp.path().join("src/a.ts");
        assert_eq!(attachment_label(Some(&guard), &inside), "src/a.ts");

        let outside = TempDir::new("attach-outside");
        let far = outside.path().join("b.ts");
        assert_eq!(
            attachment_label(Some(&guard), &far),
            far.to_string_lossy().replace('\\', "/")
        );

        // No folder open: there is no relative form to offer.
        assert_eq!(
            attachment_label(None, &inside),
            inside.to_string_lossy().replace('\\', "/")
        );
    }

    #[test]
    fn resolves_a_valid_nested_path() {
        let tmp = TempDir::new("guard-ok");
        std::fs::create_dir_all(tmp.path().join("src/nested")).unwrap();
        std::fs::write(tmp.path().join("src/nested/file.txt"), "hi").unwrap();

        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        let resolved = guard.resolve("src/nested/file.txt").unwrap();

        assert!(resolved.ends_with("file.txt"));
        assert_eq!(guard.display_relative(&resolved), "src/nested/file.txt");
    }

    #[test]
    fn rejects_parent_directory_escape() {
        let tmp = TempDir::new("guard-dotdot");
        std::fs::create_dir_all(tmp.path().join("inner")).unwrap();
        // The escaped-to path has to exist, or canonicalize fails with NotFound
        // and the containment check this test is about never runs.
        std::fs::write(tmp.path().join("outside.txt"), "secret").unwrap();
        let guard = WorkspaceGuard::new(tmp.path().join("inner")).unwrap();

        let err = guard.resolve("../outside.txt").unwrap_err();
        assert!(matches!(err, AppError::PathEscape(_)), "got {err:?}");

        let err = guard.resolve_for_write("../outside.txt").unwrap_err();
        assert!(matches!(err, AppError::PathEscape(_)), "got {err:?}");
    }

    #[test]
    fn rejects_absolute_path_escape() {
        let tmp = TempDir::new("guard-abs");
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();

        // An absolute path replaces the join base entirely. The file must exist so
        // the rejection comes from the containment check, not from canonicalize.
        let outside_dir = TempDir::new("guard-abs-outside");
        let outside = outside_dir.path().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();

        let err = guard.resolve(&outside.to_string_lossy()).unwrap_err();
        assert!(matches!(err, AppError::PathEscape(_)), "got {err:?}");
    }

    #[test]
    fn writes_inside_the_workspace_are_allowed() {
        let tmp = TempDir::new("guard-write");
        std::fs::create_dir_all(tmp.path().join("out")).unwrap();
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();

        let target = guard.resolve_for_write("out/new.txt").unwrap();
        assert_eq!(guard.display_relative(&target), "out/new.txt");
    }

    /// Creating a symlink on Windows needs developer mode or elevation, so the
    /// test reports a skip instead of failing on a locked-down machine.
    #[test]
    fn rejects_symlink_escape() {
        let tmp = TempDir::new("guard-link");
        let outside = TempDir::new("guard-link-target");
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();

        let link = workspace.join("escape");
        if create_dir_symlink(outside.path(), &link).is_err() {
            eprintln!("skipping symlink escape test: cannot create symlinks here");
            return;
        }

        let guard = WorkspaceGuard::new(&workspace).unwrap();
        let err = guard.resolve("escape").unwrap_err();
        assert!(matches!(err, AppError::PathEscape(_)), "got {err:?}");
    }

    #[cfg(windows)]
    fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    #[cfg(not(windows))]
    fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
}
