use crate::model::{Message, now};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{path::Path, sync::Mutex, time::Duration};

pub struct Store(Mutex<Connection>);

impl Store {
    pub fn put_batch(
        &self,
        documents: Vec<(&str, String, serde_json::Value)>,
    ) -> Result<(), String> {
        let mut db = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let transaction = db.transaction().map_err(|e| e.to_string())?;
        for (kind, id, data) in documents {
            transaction.execute("INSERT INTO documents(kind,id,data,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at",
                params![kind,id,data.to_string(),now()]).map_err(|e|e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        connection.execute_batch(
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
             PRAGMA user_version=1;"
        ).map_err(|e| e.to_string())?;
        Ok(Self(Mutex::new(connection)))
    }

    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<(), String> {
        let data = serde_json::to_string(value).map_err(|e| e.to_string())?;
        let mut db = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO documents(kind,id,data,updated_at) VALUES(?1,?2,?3,?4)
                    ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data, updated_at=excluded.updated_at",
                    params![kind,id,data,now()]).map_err(|e| e.to_string())?;
        if kind == "memories" {
            let value: serde_json::Value =
                serde_json::from_str(&data).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM memory_search WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO memory_search(id,content) VALUES(?1,?2)",
                params![id, value["content"].as_str().unwrap_or("")],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>, String> {
        let db = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let data: Option<String> = db
            .query_row(
                "SELECT data FROM documents WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        data.map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            .transpose()
    }

    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>, String> {
        let db = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let mut statement = db
            .prepare("SELECT data FROM documents WHERE kind=?1 ORDER BY updated_at DESC,id")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([kind], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }

    pub fn remove(&self, kind: &str, id: &str) -> Result<(), String> {
        let mut db = self.0.lock().map_err(|_| "Database lock unavailable")?;
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

    pub fn save_message(&self, message: &Message) -> Result<(), String> {
        let data = serde_json::to_string(message).map_err(|e| e.to_string())?;
        self.0
            .lock()
            .map_err(|_| "Database lock unavailable")?
            .execute(
                "INSERT INTO messages(id,conversation_id,data,created_at) VALUES(?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET data=excluded.data",
                params![
                    message.id,
                    message.conversation_id,
                    data,
                    message.created_at
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn messages(&self, conversation_id: &str) -> Result<Vec<Message>, String> {
        let db = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let mut statement = db
            .prepare("SELECT data FROM messages WHERE conversation_id=?1 ORDER BY created_at,rowid")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([conversation_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
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
            return Ok(Vec::new());
        }
        let db = self.0.lock().map_err(|_| "Database lock unavailable")?;
        let mut stmt = db.prepare("SELECT memory_search.id FROM memory_search JOIN documents d ON d.kind='memories' AND d.id=memory_search.id
            WHERE memory_search MATCH ?1 AND (json_extract(d.data,'$.orbitId')=?2 OR json_extract(d.data,'$.projectId')=?3)
            ORDER BY rank LIMIT 30").map_err(|e| e.to_string())?;
        stmt.query_map(params![query, orbit_id, project_id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .map(|r| r.map_err(|e| e.to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Memory;

    #[test]
    fn memory_index_updates_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("solar.sqlite");
        let store = Store::open(&path).unwrap();
        let mut memory = Memory {
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
}
