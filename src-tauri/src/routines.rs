use crate::{
    engine::{Engine, background_runtime},
    model::{Routine, Task, now},
};
use chrono::Utc;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

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
                    allow_host_execution: false,
                    screen_share_allowed: false,
                    cancellation: Default::default(),
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
        for mut routine in self.store.list::<Routine>("routines")? {
            if self.resetting.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            if !routine.enabled || routine.trigger == "completion" {
                continue;
            }
            let busy = self.store.list::<Task>("tasks")?.iter().any(|task| {
                task.orbit_id == routine.orbit_id
                    && ["queued", "running", "waiting"].contains(&task.status.as_str())
            });
            if busy {
                continue;
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
                routine.observed_signature = Some(signature);
                self.store.put("routines", &routine.id, &routine)?;
                changed
            } else {
                routine
                    .last_run_at
                    .as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|last| {
                        Utc::now().signed_duration_since(last).num_seconds()
                            >= i64::from(routine.interval_minutes) * 60
                    })
                    .unwrap_or(false)
            };
            if due {
                self.launch_routine(routine)?;
            }
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
            if let Err(error) = self.launch_routine(routine) {
                eprintln!("Solar completion routine: {error}");
            }
        }
    }

    fn launch_routine(self: &Arc<Self>, mut routine: Routine) -> Result<(), String> {
        self.ensure_running()?;
        routine.last_run_at = Some(now());
        self.store.put("routines", &routine.id, &routine)?;
        let conversation = self.create_conversation(
            vec![routine.orbit_id.clone()],
            routine.orbit_id,
            format!(
                "Routine · {}",
                routine.prompt.chars().take(45).collect::<String>()
            ),
            routine.project_id,
        )?;
        self.enqueue_message(&conversation.id, routine.prompt, false)?;
        Ok(())
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
