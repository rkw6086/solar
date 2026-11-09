use crate::{engine::Engine, model::*};

const MAX_ATTENTION_ITEMS: usize = 64;

#[derive(Default)]
pub(crate) struct AttentionQueue {
    items: Vec<AttentionItem>,
}

impl AttentionQueue {
    fn push(&mut self, item: AttentionItem) -> bool {
        if self.items.iter().any(|existing| existing.id == item.id) {
            return false;
        }
        let id = item.id.clone();
        if item.id.starts_with("preview:") {
            self.items
                .retain(|existing| !existing.id.starts_with("preview:"));
            self.items.insert(0, item);
        } else if item.kind == "approval" {
            let position = self
                .items
                .iter()
                .position(|entry| entry.kind != "approval")
                .unwrap_or(self.items.len());
            self.items.insert(position, item);
        } else {
            self.items.push(item);
        }
        if self.items.len() > MAX_ATTENTION_ITEMS {
            // Keep approvals ahead of informational updates without an unbounded backlog.
            let position = self
                .items
                .iter()
                .position(|entry| entry.kind != "approval" && !entry.id.starts_with("preview:"))
                .unwrap_or(self.items.len() - 1);
            self.items.remove(position);
        }
        self.items.iter().any(|entry| entry.id == id)
    }

    fn retain(&mut self, keep: impl FnMut(&AttentionItem) -> bool) -> bool {
        let before = self.items.len();
        self.items.retain(keep);
        before != self.items.len()
    }
}

impl Engine {
    pub fn attention_items(&self) -> Result<Vec<AttentionItem>, String> {
        Ok(self
            .attention
            .lock()
            .map_err(|_| "Attention state unavailable")?
            .items
            .clone())
    }

    pub(crate) fn request_attention(&self, item: AttentionItem) -> Result<(), String> {
        let added = {
            let mut queue = self
                .attention
                .lock()
                .map_err(|_| "Attention state unavailable")?;
            if !self.orbit(&item.orbit_id)?.enabled {
                return Ok(());
            }
            queue.push(item)
        };
        if added {
            self.attention_changed();
        }
        Ok(())
    }

    pub fn dismiss_attention(&self, id: &str) -> Result<(), String> {
        let removed = self
            .attention
            .lock()
            .map_err(|_| "Attention state unavailable")?
            .retain(|item| item.id != id);
        if removed {
            self.attention_changed();
        }
        Ok(())
    }

    pub(crate) fn clear_approval_attention(&self, task_id: &str) {
        let removed = self
            .attention
            .lock()
            .map(|mut queue| {
                queue.retain(|item| item.task_id != task_id || item.kind != "approval")
            })
            .unwrap_or(false);
        if removed {
            self.attention_changed();
        }
    }

    pub(crate) fn clear_orbit_attention(&self, orbit_id: &str) {
        let removed = self
            .attention
            .lock()
            .map(|mut queue| queue.retain(|item| item.orbit_id != orbit_id))
            .unwrap_or(false);
        if removed {
            self.attention_changed();
        }
    }

    fn attention_changed(&self) {
        let _ = self.events.send(AppEvent::Attention);
        self.changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str) -> AttentionItem {
        AttentionItem {
            id: id.into(),
            orbit_id: "assistant".into(),
            task_id: "task".into(),
            conversation_id: "chat".into(),
            kind: kind.into(),
            title: "Needs attention".into(),
            created_at: now(),
        }
    }

    #[test]
    fn approvals_take_priority_without_reordering_other_approvals() {
        let mut queue = AttentionQueue::default();
        queue.push(item("complete", "message"));
        queue.push(item("approval-1", "approval"));
        queue.push(item("failure", "failed"));
        queue.push(item("approval-2", "approval"));
        assert_eq!(
            queue
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["approval-1", "approval-2", "complete", "failure"]
        );
        assert!(!queue.push(item("approval-1", "approval")));
    }

    #[test]
    fn backlog_is_bounded_and_keeps_pending_approvals() {
        let mut queue = AttentionQueue::default();
        queue.push(item("approval", "approval"));
        for index in 0..200 {
            queue.push(item(&format!("message-{index}"), "message"));
        }
        assert_eq!(queue.items.len(), MAX_ATTENTION_ITEMS);
        assert_eq!(queue.items[0].id, "approval");
        assert_eq!(queue.items.last().unwrap().id, "message-199");
    }

    #[test]
    fn auditioning_another_orbit_replaces_only_the_previous_preview() {
        let mut queue = AttentionQueue::default();
        queue.push(item("approval", "approval"));
        queue.push(item("preview:one", "message"));
        queue.push(item("preview:two", "message"));
        assert_eq!(
            queue
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["preview:two", "approval"]
        );
    }

    #[test]
    fn dismissing_an_alert_never_approves_a_tool() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let approval = Approval {
            id: "a".into(),
            task_id: "task".into(),
            orbit_id: "assistant".into(),
            tool: "shell_run".into(),
            arguments: serde_json::json!({}),
        };
        engine
            .approvals
            .lock()
            .unwrap()
            .insert("a".into(), (approval, sender));
        engine
            .request_attention(item("approval:a", "approval"))
            .unwrap();
        engine.dismiss_attention("approval:a").unwrap();
        assert!(engine.attention_items().unwrap().is_empty());
        assert_eq!(engine.approvals.lock().unwrap().len(), 1);
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn finishing_an_approval_removes_only_its_alert() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        engine
            .request_attention(item("approval:a", "approval"))
            .unwrap();
        engine
            .request_attention(item("finished", "message"))
            .unwrap();
        engine.clear_approval_attention("task");
        assert_eq!(engine.attention_items().unwrap()[0].id, "finished");
    }

    #[test]
    fn old_settings_gain_short_chime_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"desktopEnabled":true,"reducedMotion":false,"maxConcurrent":2,"experimentalClaude":false}"#).unwrap();
        assert!(settings.attention_sound);
        assert_eq!(settings.attention_volume, 0.35);
    }
}
