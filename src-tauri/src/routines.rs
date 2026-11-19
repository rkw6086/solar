use crate::{
    engine::{Engine, background_runtime, validate_id},
    model::{Conversation, Routine, Task, now},
};
use chrono::{Local, Utc};
use std::str::FromStr;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

/// Five-field local-time cron; after sleep, coalesce missed runs into one run.
pub(crate) fn parse_cron(value: &str) -> Result<croner::Cron, String> {
    if value.split_whitespace().count() != 5 || value.len() > 256 {
        return Err("Use a five-part cron schedule: minute hour day month weekday".into());
    }
    croner::Cron::from_str(value).map_err(|error| format!("Invalid schedule: {error}"))
}
fn cron_due(
    value: &str,
    last_run: Option<&str>,
    now: chrono::DateTime<Utc>,
) -> Result<bool, String> {
    let cron = parse_cron(value)?;
    let Some(last) = last_run.and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok()) else {
        return Ok(false);
    };
    let next = cron
        .find_next_occurrence(&last.with_timezone(&Local), false)
        .map_err(|error| format!("Cannot calculate next scheduled run: {error}"))?;
    Ok(next.with_timezone(&Utc) <= now)
}

impl Engine {
    pub fn start_routines(self: &Arc<Self>) -> Result<(), String> {
        let runtime = background_runtime()?;
        let engine = Arc::downgrade(self);
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let Some(engine) = engine.upgrade() else {
                    break;
                };
                if let Err(error) = engine.tick_routines() {
                    eprintln!("Solar routine scheduler: {error}");
                }
            }
        });
        let engine = Arc::downgrade(self);
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                let Some(engine) = engine.upgrade() else {
                    break;
                };
                if engine.resetting.load(std::sync::atomic::Ordering::SeqCst) {
                    continue;
                }
                let has_sharing = engine
                    .sharing
                    .lock()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                if !has_sharing {
                    continue;
                }
                let context = solar_tools::ToolContext {
                    orbit_id: String::new(),
                    data_directory: engine.data_dir.clone(),
                    workspace: engine.data_dir.clone(),
                    allowed_roots: Vec::new(),
                    denied_roots: vec![],
                    allow_host_execution: false,
                    screen_share_allowed: false,
                    cancellation: Default::default(),
                    progress: None,
                };
                if let Ok(Ok(output)) = tokio::time::timeout(
                    Duration::from_secs(3),
                    engine
                        .tools
                        .execute("native_permissions", serde_json::json!({}), &context),
                )
                .await
                    && serde_json::from_str::<serde_json::Value>(&output.content)
                        .ok()
                        .and_then(|v| v["sharing"].as_bool())
                        == Some(false)
                {
                    if let Ok(mut sharing) = engine.sharing.lock() {
                        sharing.clear();
                    }
                    engine.changed();
                }
            }
        });
        Ok(())
    }

    fn tick_routines(self: &Arc<Self>) -> Result<(), String> {
        if self.resetting.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        for routine in self.store.list::<Routine>("routines")? {
            if self.resetting.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            if let Err(error) = self.tick_routine(&routine.id) {
                // A removed project or Orbit must not prevent other routines
                // from being checked during this scheduler tick.
                eprintln!("Solar routine {}: {error}", routine.id);
            }
        }
        Ok(())
    }

    fn tick_routine(self: &Arc<Self>, routine_id: &str) -> Result<(), String> {
        let _write = self
            .routine_write
            .lock()
            .map_err(|_| "Routine storage unavailable")?;
        let Some(mut routine) = self.store.get::<Routine>("routines", routine_id)? else {
            return Ok(());
        };
        if !routine.enabled || routine.trigger == "completion" {
            return Ok(());
        }
        if !self.orbit(&routine.orbit_id)?.enabled {
            return Ok(());
        }
        self.validate_routine_target(&routine)?;
        let busy = self.store.list::<Task>("tasks")?.iter().any(|task| {
            task.orbit_id == routine.orbit_id
                && ["queued", "running", "waiting"].contains(&task.status.as_str())
        });
        if busy {
            return Ok(());
        }
        let due = if routine.trigger == "folder" {
            let project = self.project(
                routine
                    .project_id
                    .as_deref()
                    .ok_or("Folder routine has no project")?,
            )?;
            let signature = directory_signature(Path::new(&project.directory))?;
            let changed = routine
                .observed_signature
                .as_ref()
                .is_some_and(|previous| previous != &signature);
            if routine.observed_signature.as_ref() != Some(&signature) {
                routine.observed_signature = Some(signature);
                self.store.put("routines", &routine.id, &routine)?;
            }
            changed
        } else if routine.trigger == "cron" {
            cron_due(&routine.cron, routine.last_run_at.as_deref(), Utc::now())?
        } else if routine.trigger == "interval" {
            routine
                .last_run_at
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|last| {
                    Utc::now().signed_duration_since(last).num_seconds()
                        >= i64::from(routine.interval_minutes) * 60
                })
                .unwrap_or(false)
        } else {
            return Err("Unknown routine trigger".into());
        };
        if due {
            self.launch_routine_locked(routine)?;
        }
        Ok(())
    }

    pub(crate) fn trigger_completion_routines(self: &Arc<Self>, orbit_id: &str) {
        let Ok(routines) = self.store.list::<Routine>("routines") else {
            return;
        };
        for routine in routines
            .into_iter()
            .filter(|r| r.enabled && r.trigger == "completion" && r.orbit_id == orbit_id)
        {
            if let Err(error) = self.launch_routine(&routine.id, true) {
                eprintln!("Solar completion routine: {error}");
            }
        }
    }

    pub fn run_routine(self: &Arc<Self>, routine_id: &str) -> Result<Conversation, String> {
        self.run_routine_checked(routine_id, |_| Ok(()))
    }

    pub(crate) fn run_routine_checked<F>(
        self: &Arc<Self>,
        routine_id: &str,
        check: F,
    ) -> Result<Conversation, String>
    where
        F: FnOnce(&Routine) -> Result<(), String>,
    {
        self.launch_routine_checked(routine_id, false, check)
    }

    fn launch_routine(
        self: &Arc<Self>,
        routine_id: &str,
        scheduled: bool,
    ) -> Result<Conversation, String> {
        self.launch_routine_checked(routine_id, scheduled, |_| Ok(()))
    }

    fn launch_routine_checked<F>(
        self: &Arc<Self>,
        routine_id: &str,
        scheduled: bool,
        check: F,
    ) -> Result<Conversation, String>
    where
        F: FnOnce(&Routine) -> Result<(), String>,
    {
        validate_id(routine_id)?;
        let _write = self
            .routine_write
            .lock()
            .map_err(|_| "Routine storage unavailable")?;
        let routine = self
            .store
            .get::<Routine>("routines", routine_id)?
            .ok_or("Routine not found")?;
        check(&routine)?;
        if scheduled && !routine.enabled {
            return Err("This routine is disabled".into());
        }
        if scheduled && routine.trigger != "completion" {
            return Err("This routine no longer runs after task completion".into());
        }
        self.launch_routine_locked(routine)
    }

    fn validate_routine_target(&self, routine: &Routine) -> Result<(), String> {
        let orbit = self.orbit(&routine.orbit_id)?;
        if !orbit.enabled {
            return Err("This Orbit is disabled".into());
        }
        if routine.trigger == "folder" && routine.project_id.is_none() {
            return Err("Folder routines need a project".into());
        }
        if let Some(project_id) = &routine.project_id {
            self.project(project_id)?;
            if !orbit.project_ids.contains(project_id) {
                return Err("Grant the Orbit access to this project first".into());
            }
        }
        Ok(())
    }

    // The caller holds routine_write so a scheduler snapshot cannot recreate a
    // deleted routine or overwrite an edit made while it was being checked.
    fn launch_routine_locked(
        self: &Arc<Self>,
        mut routine: Routine,
    ) -> Result<Conversation, String> {
        self.ensure_running()?;
        background_runtime()?;
        self.validate_routine_target(&routine)?;
        let conversation = self.create_conversation(
            vec![routine.orbit_id.clone()],
            routine.orbit_id.clone(),
            format!(
                "Routine · {}",
                routine.title.chars().take(80).collect::<String>()
            ),
            routine.project_id.clone(),
        )?;
        self.enqueue_message(&conversation.id, routine.prompt.clone(), false)?;
        routine.last_run_at = Some(now());
        self.store.put("routines", &routine.id, &routine)?;
        self.changed();
        Ok(conversation)
    }
}

fn directory_signature(path: &Path) -> Result<String, String> {
    let mut entries = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let metadata = entry.path().symlink_metadata().map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if entries.len() >= 50_000 {
                return Err(
                    "Folder routine exceeds 50,000 entries; choose a smaller project folder".into(),
                );
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            }
            let modified = metadata
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            entries.push(format!(
                "{}:{}:{modified}",
                entry.path().display(),
                metadata.len()
            ));
        }
    }
    entries.sort();
    Ok(entries.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Project;

    #[test]
    fn cron_rejects_invalid_fields_and_coalesces_due_runs() {
        assert!(parse_cron("not a schedule").is_err());
        assert!(parse_cron("* * * * * *").is_err());
        assert!(parse_cron("60 * * * *").is_err());
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-19T12:00:30Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!cron_due("* * * * *", Some("2026-11-19T12:00:10Z"), now).unwrap());
        assert!(cron_due("* * * * *", Some("2026-11-19T11:59:10Z"), now).unwrap());
        assert!(cron_due("* * * * *", Some("2026-11-18T11:59:10Z"), now).unwrap());
        assert!(!cron_due("* * * * *", None, now).unwrap());
    }

    fn routine(id: &str) -> Routine {
        Routine {
            title: String::new(),
            cron: String::new(),
            id: id.into(),
            orbit_id: "assistant".into(),
            prompt: "Summarize the latest changes.".into(),
            interval_minutes: 60,
            enabled: true,
            project_id: None,
            trigger: "interval".into(),
            last_run_at: None,
            observed_signature: None,
        }
    }

    fn project(engine: &Engine, path: &Path) {
        engine
            .save_project(Project {
                id: "project".into(),
                name: "Project".into(),
                directory: path.to_string_lossy().into(),
                description: String::new(),
            })
            .unwrap();
        let mut orbit = engine.orbit("assistant").unwrap();
        orbit.project_ids = vec!["project".into()];
        engine.save_orbit(orbit).unwrap();
    }

    #[test]
    fn saving_keeps_scheduler_metadata_and_resets_changed_folder_sources() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        project(&engine, folder.path());
        let mut draft = routine("routine");
        draft.last_run_at = Some("client timestamp".into());
        draft.observed_signature = Some("client signature".into());
        let created = engine.save_routine(draft).unwrap();
        assert_ne!(created.last_run_at.as_deref(), Some("client timestamp"));
        assert!(created.observed_signature.is_none());

        let mut stored = created.clone();
        stored.last_run_at = Some("2026-01-01T00:00:00Z".into());
        stored.observed_signature = Some("scheduler signature".into());
        engine.store.put("routines", &stored.id, &stored).unwrap();
        let mut edit = stored.clone();
        edit.prompt = "A new task.".into();
        edit.last_run_at = None;
        edit.observed_signature = Some("stale editor signature".into());
        let updated = engine.save_routine(edit).unwrap();
        assert_eq!(updated.last_run_at, stored.last_run_at);
        assert_eq!(updated.observed_signature, stored.observed_signature);

        let mut edit = updated;
        edit.trigger = "folder".into();
        edit.project_id = Some("project".into());
        let changed = engine.save_routine(edit).unwrap();
        assert_eq!(changed.last_run_at, stored.last_run_at);
        assert!(changed.observed_signature.is_none());
    }

    #[test]
    fn deleting_a_routine_is_persistent_and_does_not_restore_stale_snapshots() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        engine.save_routine(routine("routine")).unwrap();
        engine.delete_routine("routine").unwrap();
        // A scheduler which copied the ID before the deletion sees no work.
        engine.tick_routine("routine").unwrap();
        assert!(
            engine
                .delete_routine("routine")
                .unwrap_err()
                .contains("not found")
        );
        drop(engine);
        let reopened = Engine::open(root.path().into()).unwrap();
        assert!(
            reopened
                .store
                .list::<Routine>("routines")
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn manual_run_accepts_paused_routines_and_revalidates_project_access() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        project(&engine, folder.path());
        let mut paused = routine("paused");
        paused.enabled = false;
        paused.project_id = Some("project".into());
        let saved = engine.save_routine(paused).unwrap();
        let chat = engine.run_routine(&saved.id).unwrap();
        assert!(chat.title.starts_with("Routine ·"));
        assert_eq!(chat.project_id.as_deref(), Some("project"));
        assert_eq!(engine.messages(&chat.id).unwrap()[0].content, saved.prompt);
        assert_eq!(engine.store.list::<Task>("tasks").unwrap().len(), 1);

        let mut orbit = engine.orbit("assistant").unwrap();
        orbit.project_ids.clear();
        engine.save_orbit(orbit.clone()).unwrap();
        let before = engine
            .store
            .get::<Routine>("routines", &saved.id)
            .unwrap()
            .unwrap();
        assert!(
            engine
                .run_routine(&saved.id)
                .unwrap_err()
                .contains("access")
        );
        assert_eq!(engine.store.list::<Task>("tasks").unwrap().len(), 1);
        assert_eq!(
            engine
                .store
                .get::<Routine>("routines", &saved.id)
                .unwrap()
                .unwrap()
                .last_run_at,
            before.last_run_at
        );
        orbit.enabled = false;
        engine.save_orbit(orbit).unwrap();
        assert!(
            engine
                .run_routine(&saved.id)
                .unwrap_err()
                .contains("disabled")
        );
    }

    #[tokio::test]
    async fn stale_routine_does_not_block_other_due_routines() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let mut stale = routine("a-stale");
        stale.orbit_id = "removed-orbit".into();
        stale.last_run_at = Some("2020-01-01T00:00:00Z".into());
        engine.store.put("routines", &stale.id, &stale).unwrap();
        let mut due = engine.save_routine(routine("z-due")).unwrap();
        due.last_run_at = stale.last_run_at;
        engine.store.put("routines", &due.id, &due).unwrap();
        engine.tick_routines().unwrap();
        let tasks = engine.store.list::<Task>("tasks").unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].orbit_id, "assistant");
        let stored = engine
            .store
            .get::<Routine>("routines", &due.id)
            .unwrap()
            .unwrap();
        assert_ne!(stored.last_run_at, due.last_run_at);
    }

    #[tokio::test]
    async fn folder_observation_requires_access_and_only_runs_after_a_change() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        project(&engine, folder.path());
        let mut watch = routine("watch");
        watch.trigger = "folder".into();
        watch.project_id = Some("project".into());
        engine.save_routine(watch).unwrap();
        engine.tick_routines().unwrap();
        assert!(engine.store.list::<Task>("tasks").unwrap().is_empty());
        std::fs::write(folder.path().join("new.txt"), "new work").unwrap();
        let mut orbit = engine.orbit("assistant").unwrap();
        orbit.project_ids.clear();
        engine.save_orbit(orbit.clone()).unwrap();
        engine.tick_routines().unwrap();
        assert!(engine.store.list::<Task>("tasks").unwrap().is_empty());
        orbit.project_ids = vec!["project".into()];
        engine.save_orbit(orbit).unwrap();
        engine.tick_routines().unwrap();
        assert_eq!(engine.store.list::<Task>("tasks").unwrap().len(), 1);
    }

    #[cfg(not(feature = "desktop"))]
    #[test]
    fn manual_run_without_runtime_does_not_record_a_launch() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let saved = engine.save_routine(routine("routine")).unwrap();
        assert!(
            engine
                .run_routine(&saved.id)
                .unwrap_err()
                .contains("runtime")
        );
        assert!(engine.store.list::<Task>("tasks").unwrap().is_empty());
        assert!(
            engine
                .store
                .list::<Conversation>("conversations")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            engine
                .store
                .get::<Routine>("routines", &saved.id)
                .unwrap()
                .unwrap()
                .last_run_at,
            saved.last_run_at
        );
    }
}
