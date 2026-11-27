use crate::error::AppError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Team {
    pub root: PathBuf,
    pub limit: usize,
}

#[derive(Clone, Default)]
pub struct Teams(Arc<Mutex<HashMap<String, Team>>>);

impl Teams {
    pub fn begin(&self, id: String, root: PathBuf, limit: usize) -> Result<(), AppError> {
        if uuid::Uuid::parse_str(&id).is_err() || !(1..=100).contains(&limit) {
            return Err(AppError::Agent(
                "Invalid team identity or agent limit".into(),
            ));
        }
        let mut teams = self.0.lock().expect("teams mutex");
        if teams.contains_key(&id) {
            return Err(AppError::Agent("This team run already exists".into()));
        }
        teams.insert(id, Team { root, limit });
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Team, AppError> {
        self.0
            .lock()
            .expect("teams mutex")
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Agent("This team run is no longer active".into()))
    }
    pub fn finish(&self, id: &str) {
        self.0.lock().expect("teams mutex").remove(id);
    }
}

pub fn can_schedule(
    active: &[(bool, Option<&str>)],
    team: Option<&str>,
    read_only: bool,
    limit: usize,
) -> bool {
    active.len() < limit
        && active
            .iter()
            .all(|(reader, owner)| team.is_some() && *owner == team && (read_only && *reader))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_readers_overlap_and_limits_include_the_coordinator() {
        assert!(can_schedule(&[(true, Some("run"))], Some("run"), true, 2));
        assert!(!can_schedule(&[(true, Some("run"))], Some("run"), true, 1));
        assert!(!can_schedule(
            &[(false, Some("run"))],
            Some("run"),
            false,
            3
        ));
        assert!(!can_schedule(
            &[(true, Some("other"))],
            Some("run"),
            true,
            3
        ));
        assert!(!can_schedule(&[(false, None)], None, true, 3));
    }
}
