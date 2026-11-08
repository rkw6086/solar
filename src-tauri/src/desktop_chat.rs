//! The chat currently rendered in each window, independent of keyboard focus.

use crate::{
    engine::{Engine, validate_id},
    model::{AttentionItem, Conversation},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tauri::{Manager, State};

#[derive(Clone)]
struct ActiveConversation {
    id: String,
    orbit_ids: Vec<String>,
}

#[derive(Clone)]
struct WindowPresence {
    revision: u64,
    conversation: Option<ActiveConversation>,
}

#[derive(Default)]
pub(crate) struct ChatPresence(Mutex<HashMap<String, WindowPresence>>);

impl ChatPresence {
    fn update(
        &self,
        label: &str,
        revision: u64,
        conversation: Option<ActiveConversation>,
    ) -> Result<bool, String> {
        let mut windows = self
            .0
            .lock()
            .map_err(|_| "Chat visibility is unavailable")?;
        if windows
            .get(label)
            .is_some_and(|previous| revision <= previous.revision)
        {
            return Ok(false);
        }
        // Keep a revision even after unmount, so a delayed mount notification
        // cannot make a closed conversation visible again.
        windows.insert(
            label.to_owned(),
            WindowPresence {
                revision,
                conversation,
            },
        );
        Ok(true)
    }
}

#[tauri::command]
pub(crate) fn set_active_chat(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    conversation_id: Option<String>,
    revision: u64,
) -> Result<(), String> {
    let label = window.label();
    if label != "main" && !label.starts_with("chat-") {
        return Err("This window does not display a conversation".into());
    }
    if app.get_webview_window(label).is_none() {
        return Ok(());
    }
    let conversation = conversation_id
        .map(|id| -> Result<ActiveConversation, String> {
            validate_id(&id)?;
            let conversation: Conversation = engine
                .store
                .get("conversations", &id)?
                .ok_or("Conversation not found")?;
            if let Some(orbit_id) = label.strip_prefix("chat-")
                && !conversation.orbit_ids.iter().any(|id| id == orbit_id)
            {
                return Err("This conversation does not belong to the chat window".into());
            }
            Ok(ActiveConversation {
                id: conversation.id,
                orbit_ids: conversation.orbit_ids,
            })
        })
        .transpose()?;
    let changed = app
        .state::<ChatPresence>()
        .update(label, revision, conversation)?;
    if changed {
        // update() releases its mutex before presentation reads the registry.
        crate::desktop_attention::refresh(&app);
    }
    Ok(())
}

fn displays_attention(
    label: &str,
    presence: Option<&WindowPresence>,
    item: &AttentionItem,
) -> bool {
    if let Some(presence) = presence {
        return presence.conversation.as_ref().is_some_and(|conversation| {
            if item.conversation_id.is_empty() {
                conversation.orbit_ids.contains(&item.orbit_id)
            } else {
                conversation.id == item.conversation_id
            }
        });
    }
    // A newly opened popup may still be loading/creating its conversation.
    // A cleared presence is deliberately not treated as an opening popup.
    label
        .strip_prefix("chat-")
        .is_some_and(|orbit_id| item.conversation_id.is_empty() && orbit_id == item.orbit_id)
}

pub(crate) fn register_popup(app: &tauri::AppHandle, label: &str, conversation: &Conversation) {
    if let Ok(mut windows) = app.state::<ChatPresence>().0.lock() {
        windows.insert(
            label.to_owned(),
            WindowPresence {
                revision: 0,
                conversation: Some(ActiveConversation {
                    id: conversation.id.clone(),
                    orbit_ids: conversation.orbit_ids.clone(),
                }),
            },
        );
    }
}

pub(crate) fn attention_is_visible(app: &tauri::AppHandle, item: &AttentionItem) -> bool {
    let Ok(windows) = app
        .state::<ChatPresence>()
        .0
        .lock()
        .map(|state| state.clone())
    else {
        return false;
    };
    app.webview_windows().into_iter().any(|(label, window)| {
        (label == "main" || label.starts_with("chat-"))
            && window.is_visible().unwrap_or(false)
            && !window.is_minimized().unwrap_or(true)
            && displays_attention(&label, windows.get(&label), item)
    })
}

pub(crate) fn clear_window(app: &tauri::AppHandle, label: &str) {
    let removed = app
        .state::<ChatPresence>()
        .0
        .lock()
        .map(|mut windows| windows.remove(label).is_some())
        .unwrap_or(false);
    if removed {
        crate::desktop_attention::refresh(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(id: &str, orbits: &[&str]) -> ActiveConversation {
        ActiveConversation {
            id: id.into(),
            orbit_ids: orbits.iter().map(|id| (*id).into()).collect(),
        }
    }

    fn item(conversation: &str, orbit: &str) -> AttentionItem {
        AttentionItem {
            id: "alert".into(),
            orbit_id: orbit.into(),
            task_id: "task".into(),
            conversation_id: conversation.into(),
            kind: "message".into(),
            title: "Update".into(),
            created_at: crate::model::now(),
        }
    }

    #[test]
    fn late_mount_and_cleanup_requests_cannot_replace_newer_chat_state() {
        let presence = ChatPresence::default();
        assert!(
            presence
                .update("main", 2, Some(active("new", &["a"])))
                .unwrap()
        );
        assert!(!presence.update("main", 1, None).unwrap());
        assert!(!presence.update("main", 2, None).unwrap());
        assert!(presence.update("main", 4, None).unwrap());
        assert!(
            !presence
                .update("main", 3, Some(active("old", &["a"])))
                .unwrap()
        );
        let windows = presence.0.lock().unwrap();
        assert!(windows["main"].conversation.is_none());
        assert_eq!(windows["main"].revision, 4);
    }

    #[test]
    fn active_chat_matches_its_conversation_and_orbit_only_updates() {
        let presence = WindowPresence {
            revision: 1,
            conversation: Some(active("group", &["a", "b"])),
        };
        assert!(displays_attention(
            "main",
            Some(&presence),
            &item("group", "a")
        ));
        assert!(!displays_attention(
            "main",
            Some(&presence),
            &item("other", "a")
        ));
        assert!(displays_attention("main", Some(&presence), &item("", "b")));
        assert!(!displays_attention("main", Some(&presence), &item("", "c")));
        assert!(!displays_attention("main", None, &item("group", "a")));
    }

    #[test]
    fn popup_loading_fallback_ends_once_its_chat_is_known_or_cleared() {
        assert!(!displays_attention("chat-a", None, &item("first", "a")));
        assert!(displays_attention("chat-a", None, &item("", "a")));
        assert!(!displays_attention("chat-b", None, &item("first", "a")));
        let known = WindowPresence {
            revision: 1,
            conversation: Some(active("first", &["a"])),
        };
        assert!(!displays_attention(
            "chat-a",
            Some(&known),
            &item("second", "a")
        ));
        let cleared = WindowPresence {
            revision: 2,
            conversation: None,
        };
        assert!(!displays_attention(
            "chat-a",
            Some(&cleared),
            &item("first", "a")
        ));
    }
}
