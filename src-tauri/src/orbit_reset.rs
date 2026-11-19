use crate::{engine::Engine, model::Task};
use std::{sync::Arc, time::Duration};

struct ResetGuard<'a> {
    engine: &'a Engine,
    orbit_id: String,
}

impl Drop for ResetGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut resetting) = self.engine.resetting_orbits.lock() {
            resetting.remove(&self.orbit_id);
        }
    }
}

impl Engine {
    pub async fn reset_orbit_data(&self, orbit_id: &str, confirmed: bool) -> Result<(), String> {
        if !confirmed {
            return Err("Confirm clearing this Sprite's conversations and memory first.".into());
        }
        let _reset = self.reset_operations.read().await;
        self.ensure_running()?;
        self.orbit(orbit_id)?;
        // Match enqueue's lock order so no accepted message slips into reset.
        {
            let _active = self.active.lock().map_err(|_| "Task state unavailable")?;
            let mut resetting = self
                .resetting_orbits
                .lock()
                .map_err(|_| "Sprite reset state unavailable")?;
            if !resetting.insert(orbit_id.to_owned()) {
                return Err("This Sprite is already clearing its saved data.".into());
            }
        }
        let _guard = ResetGuard {
            engine: self,
            orbit_id: orbit_id.to_owned(),
        };
        let tasks: Vec<Task> = self
            .store
            .list::<Task>("tasks")?
            .into_iter()
            .filter(|task| task.orbit_id == orbit_id)
            .collect();
        for task in &tasks {
            self.stop_task(&task.id)?;
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let waiting = {
                    let active = self.active.lock().map_err(|_| "Task state unavailable")?;
                    tasks.iter().any(|task| active.contains_key(&task.id))
                };
                if !waiting {
                    break Ok::<(), String>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| "The Sprite is still stopping. Try again in a moment.")??;
        let lock = self
            .orbit_locks
            .lock()
            .map_err(|_| "Sprite lock unavailable")?
            .entry(orbit_id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _orbit = tokio::time::timeout(Duration::from_secs(10), lock.lock_owned())
            .await
            .map_err(|_| "The Sprite is busy. Stop its work and try again.")?;
        self.ensure_running()?;
        self.orbit(orbit_id)?;
        self.store.reset_orbit_data(orbit_id)?;
        self.clear_orbit_attention(orbit_id);
        self.changed();
        Ok(())
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn reset_orbit(
    window: tauri::WebviewWindow,
    engine: tauri::State<'_, Arc<Engine>>,
    orbit_id: String,
    confirmed: bool,
) -> Result<(), String> {
    if window.label() != "main" && window.label() != format!("chat-{orbit_id}") {
        return Err("Open this Sprite's chat or Solar settings to clear its data.".into());
    }
    engine.reset_orbit_data(&orbit_id, confirmed).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Memory, Message, now};

    #[tokio::test]
    async fn clearing_one_orbit_keeps_profile_files_and_shared_or_other_data() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let original = engine.orbit("assistant").unwrap();
        let mut other = original.clone();
        other.id = "other".into();
        other.name = "Other".into();
        engine.save_orbit(other).unwrap();
        let private = engine
            .create_conversation(
                vec!["assistant".into()],
                "assistant".into(),
                "Private".into(),
                None,
            )
            .unwrap();
        let shared = engine
            .create_conversation(
                vec!["assistant".into(), "other".into()],
                "assistant".into(),
                "Shared".into(),
                None,
            )
            .unwrap();
        let other_chat = engine
            .create_conversation(vec!["other".into()], "other".into(), "Other".into(), None)
            .unwrap();
        for (id, chat) in [
            ("private", &private),
            ("shared", &shared),
            ("other", &other_chat),
        ] {
            engine
                .store
                .save_message(&Message {
                    id: id.into(),
                    conversation_id: chat.id.clone(),
                    orbit_id: Some("assistant".into()),
                    role: "assistant".into(),
                    content: id.into(),
                    created_at: now(),
                    tool_activity: None,
                })
                .unwrap();
        }
        for owner in ["assistant", "other"] {
            engine
                .save_memory(Memory {
                    orbit_ids: Vec::new(),
                    project_ids: Vec::new(),
                    id: format!("memory-{owner}"),
                    orbit_id: Some(owner.into()),
                    project_id: None,
                    content: "Remember this".into(),
                    created_at: now(),
                })
                .unwrap();
        }
        let file = engine.workspace("assistant").unwrap().join("keep.txt");
        std::fs::write(&file, "keep").unwrap();
        assert!(engine.reset_orbit_data("assistant", false).await.is_err());
        assert_eq!(engine.memories(Some("assistant"), None).unwrap().len(), 1);
        engine.reset_orbit_data("assistant", true).await.unwrap();
        assert!(engine.memories(Some("assistant"), None).unwrap().is_empty());
        assert_eq!(engine.memories(Some("other"), None).unwrap().len(), 1);
        let state = engine.bootstrap().unwrap();
        assert!(state.conversations.iter().all(|chat| chat.id != private.id));
        assert!(state.conversations.iter().any(|chat| chat.id == shared.id));
        assert_eq!(engine.messages(&shared.id).unwrap().len(), 1);
        assert_eq!(engine.messages(&other_chat.id).unwrap().len(), 1);
        assert_eq!(
            engine.orbit("assistant").unwrap().instructions,
            original.instructions
        );
        assert_eq!(std::fs::read_to_string(file).unwrap(), "keep");
    }

    #[tokio::test]
    async fn reset_guard_rejects_new_messages_without_persisting_work() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let chat = engine
            .create_conversation(
                vec!["assistant".into()],
                "assistant".into(),
                "Private".into(),
                None,
            )
            .unwrap();
        engine
            .resetting_orbits
            .lock()
            .unwrap()
            .insert("assistant".into());
        assert!(
            engine
                .send_message(&chat.id, "Do not enqueue this".into())
                .unwrap_err()
                .contains("clearing")
        );
        assert!(engine.messages(&chat.id).unwrap().is_empty());
        assert!(engine.bootstrap().unwrap().tasks.is_empty());
        engine.resetting_orbits.lock().unwrap().clear();
        engine.reset_orbit_data("assistant", true).await.unwrap();
        assert!(engine.resetting_orbits.lock().unwrap().is_empty());
    }
}
