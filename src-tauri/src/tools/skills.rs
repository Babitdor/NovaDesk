//! Skills: the SKILL.md folders Nova mounts, listed honestly.
//!
//! The pane used to say nothing about skills, and a harness saw only whatever
//! its own installer had written. This reads the same directories the agents do
//! and reports the description from each frontmatter, so a tool and the pane can
//! both answer "what can this workspace do" without inventing entries.
//!
//! Only the first 8 KiB of a SKILL.md is read: frontmatter is at the top, and a
//! skill with a large body should not be loaded to learn its one-line summary.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// A skill directory larger than this is not a skill file anymore.
const MAX_SKILL_BYTES: u64 = 8 * 1024;

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Every root a harness in this app reads, with the label the pane shows.
fn roots(workspace: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    let mut roots = Vec::new();
    if let Some(workspace) = workspace {
        roots.push((".nova/skills", workspace.join(".nova").join("skills")));
    }
    if let Some(home) = home() {
        roots.push(("~/.nova/skills", home.join(".nova").join("skills")));
        roots.push(("~/.claude/skills", home.join(".claude").join("skills")));
        roots.push(("~/.agents/skills", home.join(".agents").join("skills")));
    }
    roots
}

/// The two frontmatter fields a list needs. Deliberately a line reader rather
/// than a YAML dependency: name and description are plain scalars, and the
/// alternative is a parser for a document this code never has to understand.
fn frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let mut name = None;
    let mut description = None;
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().trim_matches(['"', '\'']).to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().trim_matches(['"', '\'']).to_string());
        } else if line.starts_with('>') || line.starts_with('|') {
            // A folded description is continued on the following lines; the
            // first line is the part a list can show.
            continue;
        }
    }
    (name, description)
}

pub fn list(workspace: Option<&Path>) -> Value {
    let mut skills: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (label, root) in roots(workspace) {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let manifest = path.join("SKILL.md");
            if !path.is_dir() || !manifest.is_file() {
                continue;
            }
            let directory = entry.file_name().to_string_lossy().into_owned();
            if seen.iter().any(|name| name == &directory) {
                // The project's copy of a skill wins: it is the one the user
                // edited, and listing both would look like two skills.
                continue;
            }
            let text = std::fs::metadata(&manifest)
                .ok()
                .filter(|meta| meta.len() <= MAX_SKILL_BYTES)
                .and_then(|_| std::fs::read_to_string(&manifest).ok())
                .unwrap_or_default();
            let (name, description) = frontmatter(&text);
            seen.push(directory.clone());
            skills.push(json!({
                "name": name.unwrap_or_else(|| directory.clone()),
                "directory": directory,
                "root": label,
                "description": description.unwrap_or_default(),
            }));
        }
    }
    skills.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    json!({ "skills": skills, "count": skills.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    #[test]
    fn reads_name_and_description_from_frontmatter() {
        let (name, description) = frontmatter(
            "---\nname: \"old-coder\"\ndescription: Evidence-first development\nversion: 1\n---\nbody\n",
        );
        assert_eq!(name.as_deref(), Some("old-coder"));
        assert_eq!(description.as_deref(), Some("Evidence-first development"));
    }

    #[test]
    fn a_file_without_frontmatter_has_no_summary() {
        assert_eq!(frontmatter("# Title\nbody"), (None, None));
    }

    #[test]
    fn the_project_skill_wins_over_a_home_skill_of_the_same_name() {
        let root = TempDir::new("skills-project");
        let dir = root.path().join(".nova/skills/demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo\ndescription: local\n---\n",
        )
        .unwrap();
        let listed = list(Some(root.path()));
        let skills = listed["skills"].as_array().unwrap();
        let demo = skills.iter().find(|skill| skill["name"] == "demo").unwrap();
        assert_eq!(demo["description"], "local");
        assert_eq!(demo["root"], ".nova/skills");
    }
}
