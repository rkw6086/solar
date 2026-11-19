use crate::{
    model::{Message, now},
    profile_config::{CONFIG_KINDS, ProfileConfig, secure_directory, secure_file},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub struct Store(Mutex<StoreState>);

struct StoreState {
    main: Connection,
    profile: Option<ProfileConfig>,
    orbit_dbs: BTreeMap<String, Connection>,
}

#[derive(Clone)]
struct Document {
    id: String,
    data: String,
    updated_at: String,
}

fn private_kind(kind: &str) -> bool {
    matches!(
        kind,
        "memories" | "runs" | "history" | "history_origin" | "last_user"
    )
}

fn open_connection(path: &Path, private: bool) -> Result<Connection, String> {
    if private {
        secure_file(path)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            secure_file(Path::new(&sidecar))?;
        }
    }
    let connection = Connection::open(path).map_err(|e| e.to_string())?;
    if private {
        secure_file(path)?;
    }
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS documents (
             kind TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL,
             updated_at TEXT NOT NULL, PRIMARY KEY(kind,id));
         CREATE TABLE IF NOT EXISTS messages (
             id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, data TEXT NOT NULL,
             created_at TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id,created_at);
         CREATE VIRTUAL TABLE IF NOT EXISTS memory_search USING fts5(id UNINDEXED, content);
         PRAGMA user_version=1;",
        )
        .map_err(|e| e.to_string())?;
    Ok(connection)
}

fn query_document(db: &Connection, kind: &str, id: &str) -> Result<Option<Document>, String> {
    db.query_row(
        "SELECT id,data,updated_at FROM documents WHERE kind=?1 AND id=?2",
        params![kind, id],
        |r| {
            Ok(Document {
                id: r.get(0)?,
                data: r.get(1)?,
                updated_at: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn query_documents(db: &Connection, kind: &str) -> Result<Vec<Document>, String> {
    let mut statement = db
        .prepare(
            "SELECT id,data,updated_at FROM documents WHERE kind=?1 ORDER BY updated_at DESC,id",
        )
        .map_err(|e| e.to_string())?;
    statement
        .query_map([kind], |r| {
            Ok(Document {
                id: r.get(0)?,
                data: r.get(1)?,
                updated_at: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

fn put_documents(db: &mut Connection, documents: Vec<(&str, String, Value)>) -> Result<(), String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    for (kind, id, value) in documents {
        tx.execute("INSERT INTO documents(kind,id,data,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at", params![kind,id,value.to_string(),now()]).map_err(|e| e.to_string())?;
        if kind == "memories" {
            tx.execute("DELETE FROM memory_search WHERE id=?1", [&id])
                .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO memory_search(id,content) VALUES(?1,?2)",
                params![id, value["content"].as_str().unwrap_or("")],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

fn remove_document(db: &mut Connection, kind: &str, id: &str) -> Result<(), String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM documents WHERE kind=?1 AND id=?2",
        params![kind, id],
    )
    .map_err(|e| e.to_string())?;
    if kind == "memories" {
        tx.execute("DELETE FROM memory_search WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

impl StoreState {
    fn connection(&mut self, owner: Option<&str>) -> Result<&mut Connection, String> {
        let Some(owner) = owner else {
            return Ok(&mut self.main);
        };
        if !self.orbit_dbs.contains_key(owner) {
            let profile = self
                .profile
                .as_ref()
                .ok_or("Sprite storage requires a Solar profile")?;
            let path = profile.orbit_dir(owner)?.join("orbit.sqlite");
            self.orbit_dbs
                .insert(owner.into(), open_connection(&path, true)?);
        }
        self.orbit_dbs
            .get_mut(owner)
            .ok_or_else(|| "Sprite database is unavailable".into())
    }

    fn orbit_ids(&self) -> Vec<String> {
        self.profile
            .as_ref()
            .map(|p| p.locations().keys().cloned().collect())
            .unwrap_or_default()
    }

    fn owner(&self, kind: &str, id: &str, data: &Value) -> Result<Option<String>, String> {
        let Some(profile) = &self.profile else {
            return Ok(None);
        };
        let owner = match kind {
            "memories" => data
                .get("orbitId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            "runs" => {
                let task = query_document(&self.main, "tasks", id)?
                    .ok_or("Private run has no task owner")?;
                serde_json::from_str::<Value>(&task.data).map_err(|e| e.to_string())?["orbitId"]
                    .as_str()
                    .map(str::to_owned)
            }
            "history" | "history_origin" | "last_user" => {
                let candidates: Vec<_> = profile
                    .locations()
                    .keys()
                    .filter(|orbit_id| id.ends_with(&format!("_{orbit_id}")))
                    .cloned()
                    .collect();
                if candidates.len() == 1 {
                    candidates.into_iter().next()
                } else {
                    let mut matches = Vec::new();
                    for orbit_id in candidates {
                        let conversation_id = id.strip_suffix(&format!("_{orbit_id}")).unwrap();
                        if let Some(conversation) =
                            query_document(&self.main, "conversations", conversation_id)?
                        {
                            let value: Value = serde_json::from_str(&conversation.data)
                                .map_err(|e| e.to_string())?;
                            if value["orbitIds"].as_array().is_some_and(|ids| {
                                ids.iter().any(|id| id.as_str() == Some(&orbit_id))
                            }) {
                                matches.push(orbit_id);
                            }
                        }
                    }
                    if matches.len() > 1 {
                        return Err("Ambiguous Sprite history key".into());
                    }
                    matches.into_iter().next()
                }
            }
            _ => return Ok(None),
        };
        if let Some(owner) = &owner {
            if !profile.locations().contains_key(owner) {
                return Err("Private record references an unknown Sprite".into());
            }
        } else if kind != "memories" {
            return Err("Cannot determine the Sprite for this private record".into());
        }
        Ok(owner)
    }

    fn message_owner(&self, conversation_id: &str) -> Result<Option<String>, String> {
        if self.profile.is_none() {
            return Ok(None);
        }
        let Some(conversation) = query_document(&self.main, "conversations", conversation_id)?
        else {
            return Ok(None);
        };
        let value: Value = serde_json::from_str(&conversation.data).map_err(|e| e.to_string())?;
        let Some(orbits) = value["orbitIds"].as_array() else {
            return Err("Conversation has invalid Sprite membership".into());
        };
        if orbits.len() != 1 {
            return Ok(None);
        }
        let owner = orbits[0]
            .as_str()
            .ok_or("Conversation has invalid Sprite membership")?;
        if !self
            .profile
            .as_ref()
            .unwrap()
            .locations()
            .contains_key(owner)
        {
            return Err("Conversation references an unknown Sprite".into());
        }
        Ok(Some(owner.into()))
    }

    fn sync_configuration(&mut self) -> Result<(), String> {
        let profile = self
            .profile
            .as_ref()
            .ok_or("Solar profile is unavailable")?;
        let documents = profile.documents()?;
        let locations = serde_json::to_value(profile.locations()).map_err(|e| e.to_string())?;
        let tx = self.main.transaction().map_err(|e| e.to_string())?;
        for kind in CONFIG_KINDS {
            tx.execute("DELETE FROM documents WHERE kind=?1", [kind])
                .map_err(|e| e.to_string())?;
        }
        for (kind, id, data) in documents {
            tx.execute(
                "INSERT INTO documents(kind,id,data,updated_at) VALUES(?1,?2,?3,?4)",
                params![kind, id, data.to_string(), now()],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute("INSERT INTO documents(kind,id,data,updated_at) VALUES('meta','orbit_directories',?1,?2) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at",params![locations.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    fn detach_memory_audience(&mut self, kind: &str, removed_id: &str) -> Result<(), String> {
        if !["orbits", "projects"].contains(&kind) {
            return Ok(());
        }
        for row in self.documents("memories")? {
            let mut memory: crate::model::Memory =
                serde_json::from_str(&row.data).map_err(|e| e.to_string())?;
            let mut orbits = memory.orbit_ids();
            let mut projects = memory.project_ids();
            let audience = if kind == "orbits" {
                &mut orbits
            } else {
                &mut projects
            };
            if !audience.iter().any(|id| id == removed_id) {
                continue;
            }
            audience.retain(|id| id != removed_id);
            if orbits.is_empty() && projects.is_empty() {
                // Losing the final attachment must never turn a private fact public.
                remove_document(&mut self.main, "memories", &memory.id)?;
                for owner in self.orbit_ids() {
                    remove_document(self.connection(Some(&owner))?, "memories", &memory.id)?;
                }
                continue;
            }
            memory.orbit_id = (orbits.len() == 1).then(|| orbits[0].clone());
            memory.project_id = (projects.len() == 1).then(|| projects[0].clone());
            memory.orbit_ids = orbits;
            memory.project_ids = projects;
            let data = serde_json::to_value(&memory).map_err(|e| e.to_string())?;
            let owner = self.owner("memories", &memory.id, &data)?;
            // Preserve the remaining readers' copy before removing old locations.
            put_documents(
                self.connection(owner.as_deref())?,
                vec![("memories", memory.id.clone(), data)],
            )?;
            if owner.is_some() {
                remove_document(&mut self.main, "memories", &memory.id)?;
            }
            for old_owner in self.orbit_ids() {
                if owner.as_deref() != Some(&old_owner) {
                    remove_document(self.connection(Some(&old_owner))?, "memories", &memory.id)?;
                }
            }
        }
        Ok(())
    }

    fn document(&mut self, kind: &str, id: &str) -> Result<Option<Document>, String> {
        let mut result = query_document(&self.main, kind, id)?;
        if private_kind(kind) {
            for owner in self.orbit_ids() {
                if let Some(document) = query_document(self.connection(Some(&owner))?, kind, id)?
                    && result
                        .as_ref()
                        .is_none_or(|old| old.updated_at < document.updated_at)
                {
                    result = Some(document);
                }
            }
        }
        Ok(result)
    }

    fn documents(&mut self, kind: &str) -> Result<Vec<Document>, String> {
        let mut result = query_documents(&self.main, kind)?;
        if private_kind(kind) {
            for owner in self.orbit_ids() {
                result.extend(query_documents(self.connection(Some(&owner))?, kind)?);
            }
        }
        result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        let mut seen = HashSet::new();
        result.retain(|row| seen.insert(row.id.clone()));
        Ok(result)
    }
}

impl Store {
    /// Standalone database mode, retained for fixtures and small local stores.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        Ok(Self(Mutex::new(StoreState {
            main: open_connection(path, false)?,
            profile: None,
            orbit_dbs: BTreeMap::new(),
        })))
    }

    /// Open the current ~/.solar-style layout. This does not import legacy app
    /// directories. TOML profiles replace their SQLite configuration cache.
    pub fn open_profile(root: &Path) -> Result<Self, String> {
        secure_directory(root)?;
        let mut state = StoreState {
            main: open_connection(&root.join("solar.sqlite"), true)?,
            profile: None,
            orbit_dbs: BTreeMap::new(),
        };
        let locations = query_document(&state.main, "meta", "orbit_directories")?
            .map(|row| {
                serde_json::from_str::<BTreeMap<String, String>>(&row.data)
                    .map_err(|e| e.to_string())
            })
            .transpose()?
            .unwrap_or_default();
        state.profile = Some(ProfileConfig::open(root, locations)?);
        state.sync_configuration()?;
        for owner in state.orbit_ids() {
            state.connection(Some(&owner))?;
        }
        Ok(Self(Mutex::new(state)))
    }

    pub fn orbit_dir(&self, orbit_id: &str) -> Result<PathBuf, String> {
        self.0
            .lock()
            .map_err(|_| "Database lock unavailable")?
            .profile
            .as_ref()
            .ok_or("Sprite storage requires a Solar profile")?
            .orbit_dir(orbit_id)
    }

    pub fn put_batch(&self, documents: Vec<(&str, String, Value)>) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        if state.profile.is_some()
            && documents
                .iter()
                .any(|(kind, _, _)| CONFIG_KINDS.contains(kind))
        {
            return Err("Configuration files must be updated individually".into());
        }
        let mut target: Option<Option<String>> = None;
        for (kind, id, data) in &documents {
            let owner = state.owner(kind, id, data)?;
            if target.as_ref().is_some_and(|previous| previous != &owner) {
                return Err("A transaction cannot span multiple Sprite databases".into());
            }
            target = Some(owner);
        }
        let owner = target.flatten();
        put_documents(state.connection(owner.as_deref())?, documents)
    }

    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<(), String> {
        let data = serde_json::to_value(value).map_err(|e| e.to_string())?;
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        if CONFIG_KINDS.contains(&kind) && state.profile.is_some() {
            state.profile.as_mut().unwrap().put(kind, id, data)?;
            state.sync_configuration()?;
            if kind == "orbits" {
                state.connection(Some(id))?;
            }
            return Ok(());
        }
        let owner = state.owner(kind, id, &data)?;
        put_documents(
            state.connection(owner.as_deref())?,
            vec![(kind, id.into(), data)],
        )?;
        // Reassigning a memory keeps one authoritative copy. Copy first, then
        // remove stale locations so a write failure never destroys its content.
        if kind == "memories" {
            if owner.is_some() {
                remove_document(&mut state.main, kind, id)?;
            }
            for old_owner in state.orbit_ids() {
                if owner.as_deref() != Some(&old_owner) {
                    remove_document(state.connection(Some(&old_owner))?, kind, id)?;
                }
            }
        }
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>, String> {
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        state
            .document(kind, id)?
            .map(|row| serde_json::from_str(&row.data).map_err(|e| e.to_string()))
            .transpose()
    }

    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>, String> {
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        state
            .documents(kind)?
            .into_iter()
            .map(|row| serde_json::from_str(&row.data).map_err(|e| e.to_string()))
            .collect()
    }

    pub fn remove(&self, kind: &str, id: &str) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        state.detach_memory_audience(kind, id)?;
        if CONFIG_KINDS.contains(&kind) && state.profile.is_some() {
            state.profile.as_mut().unwrap().remove(kind, id)?;
            return state.sync_configuration();
        }
        remove_document(&mut state.main, kind, id)?;
        if private_kind(kind) {
            for owner in state.orbit_ids() {
                remove_document(state.connection(Some(&owner))?, kind, id)?;
            }
        }
        Ok(())
    }

    pub fn save_message(&self, message: &Message) -> Result<(), String> {
        let data = serde_json::to_string(message).map_err(|e| e.to_string())?;
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let owner = state.message_owner(&message.conversation_id)?;
        state.connection(owner.as_deref())?.execute(
            "INSERT INTO messages(id,conversation_id,data,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            params![message.id,message.conversation_id,data,message.created_at]).map(|_|()).map_err(|e|e.to_string())
    }

    pub fn messages(&self, conversation_id: &str) -> Result<Vec<Message>, String> {
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let owner = state.message_owner(conversation_id)?;
        let mut statement = state
            .connection(owner.as_deref())?
            .prepare("SELECT data FROM messages WHERE conversation_id=?1 ORDER BY created_at,rowid")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([conversation_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }

    pub fn matching_memories(
        &self,
        query: &str,
        orbit_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let query = query
            .split_whitespace()
            .take(12)
            .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        if query.is_empty() {
            return Ok(vec![]);
        }
        fn search(db: &Connection, query: &str) -> Result<Vec<String>, String> {
            let mut stmt = db.prepare("SELECT memory_search.id FROM memory_search JOIN documents d ON d.kind='memories' AND d.id=memory_search.id WHERE memory_search MATCH ?1 ORDER BY rank LIMIT 100").map_err(|e|e.to_string())?;
            stmt.query_map(params![query], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())
        }
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let mut ids = search(&state.main, &query)?;
        if state.profile.is_some()
            && let Some(owner) = orbit_id
        {
            ids.extend(search(state.connection(Some(owner))?, &query)?);
        }
        let projects: Vec<String> = project_id.into_iter().map(str::to_owned).collect();
        ids.retain(|id| {
            state
                .document("memories", id)
                .ok()
                .flatten()
                .and_then(|row| serde_json::from_str::<crate::model::Memory>(&row.data).ok())
                .is_some_and(|memory| memory.visible_to(orbit_id.unwrap_or(""), &projects))
        });
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        ids.truncate(30);
        Ok(ids)
    }

    /// Clear one Orbit's learned/chat data without changing its profile or files.
    /// The caller must stop its active work and hold its execution lock first.
    pub(crate) fn reset_orbit_data(&self, orbit_id: &str) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let profile = state
            .profile
            .as_ref()
            .ok_or("Sprite reset requires a Solar profile")?;
        if !profile.locations().contains_key(orbit_id) {
            return Err("Sprite not found".into());
        }
        let private_chats: Vec<String> = query_documents(&state.main, "conversations")?
            .into_iter()
            .filter_map(|row| {
                let value: Value = serde_json::from_str(&row.data).ok()?;
                let members = value["orbitIds"].as_array()?;
                (members.len() == 1 && members[0].as_str() == Some(orbit_id)).then_some(row.id)
            })
            .collect();
        // Orbit-private tables contain no other Orbit's or shared project data.
        let private = state.connection(Some(orbit_id))?;
        let tx = private.transaction().map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM documents WHERE kind IN ('memories','runs','history','history_origin','last_user')", [])
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM messages", [])
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM memory_search", [])
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;

        // Keep shared group messages/conversations and other Orbits' activity.
        let tx = state
            .main
            .transaction()
            .map_err(|error| error.to_string())?;
        for conversation_id in private_chats {
            tx.execute(
                "DELETE FROM documents WHERE kind='conversations' AND id=?1",
                [&conversation_id],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "DELETE FROM messages WHERE conversation_id=?1",
                [&conversation_id],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.execute(
            "DELETE FROM documents WHERE kind='tasks' AND json_extract(data,'$.orbitId')=?1",
            [orbit_id],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Memory, Orbit};

    fn orbit(id: &str, name: &str) -> Orbit {
        Orbit {
            id: id.into(),
            name: name.into(),
            role: "Assistant".into(),
            tasks: String::new(),
            personality: "Calm".into(),
            instructions: String::new(),
            color: "#fff".into(),
            avatar_seed: None,
            model: String::new(),
            provider_id: String::new(),
            autonomy: "ask".into(),
            tool_grants: vec![],
            project_ids: vec![],
            x: None,
            y: None,
            enabled: true,
        }
    }

    #[test]
    fn memory_index_updates_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("solar.sqlite");
        let store = Store::open(&path).unwrap();
        let mut memory = Memory {
            orbit_ids: Vec::new(),
            project_ids: Vec::new(),
            id: "one".into(),
            orbit_id: Some("a".into()),
            project_id: None,
            content: "solar energy".into(),
            created_at: now(),
        };
        store.put("memories", &memory.id, &memory).unwrap();
        assert_eq!(
            store.matching_memories("solar", Some("a"), None).unwrap(),
            ["one"]
        );
        memory.content = "lunar geology".into();
        store.put("memories", &memory.id, &memory).unwrap();
        assert!(
            store
                .matching_memories("solar", Some("a"), None)
                .unwrap()
                .is_empty()
        );
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.matching_memories("lunar", Some("a"), None).unwrap(),
            ["one"]
        );
        store.remove("memories", "one").unwrap();
        assert!(
            store
                .matching_memories("lunar", Some("a"), None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn orbit_profiles_are_authoritative_and_directories_stay_stable() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_profile(dir.path()).unwrap();
        store.put("orbits", "a", &orbit("a", "Nova")).unwrap();
        store.put("orbits", "b", &orbit("b", "NOVA")).unwrap();
        let a = store.orbit_dir("a").unwrap();
        let b = store.orbit_dir("b").unwrap();
        assert_ne!(a, b);
        assert!(a.join("config.toml").exists());
        assert!(a.join("orbit.sqlite").exists());
        store.put("orbits", "a", &orbit("a", "Renamed")).unwrap();
        assert_eq!(store.orbit_dir("a").unwrap(), a);
        drop(store);
        let path = a.join("config.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("Renamed", "Hand edited");
        std::fs::write(&path, text).unwrap();
        let store = Store::open_profile(dir.path()).unwrap();
        assert_eq!(
            store.get::<Orbit>("orbits", "a").unwrap().unwrap().name,
            "Hand edited"
        );
    }

    #[test]
    fn private_records_live_in_their_orbit_database_and_batches_stay_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_profile(dir.path()).unwrap();
        store.put("orbits", "a", &orbit("a", "Nova")).unwrap();
        store.put("orbits", "b", &orbit("b", "Lyra")).unwrap();
        let memory = Memory {
            orbit_ids: Vec::new(),
            project_ids: Vec::new(),
            id: "one".into(),
            orbit_id: Some("a".into()),
            project_id: None,
            content: "private lunar geology".into(),
            created_at: now(),
        };
        store.put("memories", "one", &memory).unwrap();
        store
            .put("tasks", "run-a", &serde_json::json!({"orbitId":"a"}))
            .unwrap();
        store
            .put_batch(vec![
                (
                    "runs",
                    "run-a".into(),
                    serde_json::json!({"history_key":"chat_a"}),
                ),
                ("history", "chat_a".into(), serde_json::json!(["private"])),
            ])
            .unwrap();
        assert!(
            store
                .put_batch(vec![
                    ("history", "bad_a".into(), serde_json::json!([])),
                    ("history", "bad_b".into(), serde_json::json!([]))
                ])
                .is_err()
        );
        assert!(store.get::<Value>("history", "bad_a").unwrap().is_none());
        let global = Store::open(&dir.path().join("solar.sqlite")).unwrap();
        assert!(global.get::<Memory>("memories", "one").unwrap().is_none());
        assert!(global.get::<Value>("history", "chat_a").unwrap().is_none());
        let private = Store::open(&store.orbit_dir("a").unwrap().join("orbit.sqlite")).unwrap();
        assert_eq!(
            private
                .get::<Memory>("memories", "one")
                .unwrap()
                .unwrap()
                .content,
            memory.content
        );
        assert_eq!(
            store.matching_memories("lunar", Some("a"), None).unwrap(),
            ["one"]
        );
        assert!(
            store
                .matching_memories("lunar", Some("b"), None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn invalid_toml_is_preserved_and_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[invalid").unwrap();
        assert!(Store::open_profile(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "[invalid");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_config_and_orbit_directories_are_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        symlink(target.path(), dir.path().join("orbits")).unwrap();
        assert!(Store::open_profile(dir.path()).is_err());
    }
    #[test]
    fn single_orbit_chats_are_private_and_group_chats_are_shared() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_profile(dir.path()).unwrap();
        store.put("orbits", "a", &orbit("a", "Sol")).unwrap();
        store.put("orbits", "b", &orbit("b", "Lyra")).unwrap();
        store
            .put(
                "conversations",
                "private",
                &serde_json::json!({"orbitIds":["a"]}),
            )
            .unwrap();
        store
            .put(
                "conversations",
                "shared",
                &serde_json::json!({"orbitIds":["a","b"]}),
            )
            .unwrap();
        for conversation_id in ["private", "shared"] {
            store
                .save_message(&Message {
                    id: conversation_id.into(),
                    conversation_id: conversation_id.into(),
                    orbit_id: None,
                    role: "user".into(),
                    content: "hello".into(),
                    created_at: now(),
                    tool_activity: None,
                })
                .unwrap();
            assert_eq!(store.messages(conversation_id).unwrap().len(), 1);
        }
        let global = Store::open(&dir.path().join("solar.sqlite")).unwrap();
        assert!(global.messages("private").unwrap().is_empty());
        assert_eq!(global.messages("shared").unwrap().len(), 1);
        let private = Store::open(&store.orbit_dir("a").unwrap().join("orbit.sqlite")).unwrap();
        assert_eq!(private.messages("private").unwrap().len(), 1);
        assert!(private.messages("shared").unwrap().is_empty());
    }

    #[test]
    fn malformed_or_externally_edited_configuration_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_profile(dir.path()).unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[broken").unwrap();
        assert!(
            store
                .put("settings", "app", &crate::model::Settings::default())
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[broken");
        std::fs::remove_file(&path).unwrap();
        drop(store);
        let store = Store::open_profile(dir.path()).unwrap();
        let changed = std::fs::read_to_string(&path)
            .unwrap()
            .replace("maxConcurrent = 2", "maxConcurrent = 3");
        std::fs::write(&path, &changed).unwrap();
        assert!(
            store
                .put("settings", "app", &crate::model::Settings::default())
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
    }

    #[test]
    fn ambiguous_history_keys_are_rejected_and_mcp_namespaces_cannot_collide() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_profile(dir.path()).unwrap();
        store.put("orbits", "qa", &orbit("qa", "QA")).unwrap();
        store
            .put("orbits", "a_qa", &orbit("a_qa", "Other QA"))
            .unwrap();
        assert!(
            store
                .put("history", "chat_a_qa", &serde_json::json!([]))
                .is_err()
        );
        store
            .put(
                "conversations",
                "chat",
                &serde_json::json!({"orbitIds":["a_qa"]}),
            )
            .unwrap();
        store
            .put("history", "chat_a_qa", &serde_json::json!(["safe"]))
            .unwrap();
        let server = |id: &str| crate::model::McpServer {
            id: id.into(),
            name: "MCP".into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: Some("http://localhost/".into()),
            enabled: false,
            connected: false,
            has_credentials: false,
            revision: 1,
        };
        store.put("mcp", "My-Server", &server("My-Server")).unwrap();
        assert!(store.put("mcp", "my_server", &server("my_server")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_database_files_are_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), dir.path().join("solar.sqlite")).unwrap();
        assert!(Store::open_profile(dir.path()).is_err());
        std::fs::remove_file(dir.path().join("solar.sqlite")).unwrap();
        let store = Store::open_profile(dir.path()).unwrap();
        store.put("orbits", "a", &orbit("a", "Sol")).unwrap();
        let path = store.orbit_dir("a").unwrap().join("orbit.sqlite");
        drop(store);
        std::fs::remove_file(&path).unwrap();
        symlink(outside.path(), &path).unwrap();
        assert!(Store::open_profile(dir.path()).is_err());
    }
    #[test]
    fn deleting_audiences_preserves_remaining_readers_without_making_memories_public() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_profile(root.path()).unwrap();
        store.put("orbits", "a", &orbit("a", "Nova")).unwrap();
        store.put("orbits", "b", &orbit("b", "Lyra")).unwrap();
        for project in ["p1", "p2"] {
            store.put("projects", project, &serde_json::json!({"id":project,"name":project,"directory":root.path().join(project),"description":""})).unwrap();
        }
        let memory = |id: &str, orbits: Vec<String>, projects: Vec<String>| Memory {
            id: id.into(),
            orbit_id: (orbits.len() == 1).then(|| orbits[0].clone()),
            project_id: (projects.len() == 1).then(|| projects[0].clone()),
            orbit_ids: orbits,
            project_ids: projects,
            content: "Kept private".into(),
            created_at: now(),
        };
        for item in [
            memory("private", vec!["a".into()], vec![]),
            memory("shared", vec!["a".into(), "b".into()], vec![]),
            memory("project-shared", vec![], vec!["p1".into(), "p2".into()]),
            memory("global", vec![], vec![]),
        ] {
            store.put("memories", &item.id, &item).unwrap();
        }
        store.remove("orbits", "a").unwrap();
        assert!(
            store
                .get::<Memory>("memories", "private")
                .unwrap()
                .is_none()
        );
        let shared = store.get::<Memory>("memories", "shared").unwrap().unwrap();
        assert_eq!(shared.orbit_ids(), vec!["b"]);
        assert!(!shared.is_system());
        let main = Store::open(&root.path().join("solar.sqlite")).unwrap();
        assert!(main.get::<Memory>("memories", "shared").unwrap().is_none());
        store.remove("orbits", "b").unwrap();
        assert!(store.get::<Memory>("memories", "shared").unwrap().is_none());
        store.remove("projects", "p1").unwrap();
        let shared = store
            .get::<Memory>("memories", "project-shared")
            .unwrap()
            .unwrap();
        assert_eq!(shared.project_ids(), vec!["p2"]);
        assert!(!shared.is_system());
        store.remove("projects", "p2").unwrap();
        assert!(
            store
                .get::<Memory>("memories", "project-shared")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get::<Memory>("memories", "global")
                .unwrap()
                .unwrap()
                .is_system()
        );
    }
}
