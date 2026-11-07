//! Reset is prepared while the app is running and applied before opening the
//! database on the next launch. No live SQLite file is removed.

use crate::engine::Engine;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};

const MARKER: &str = ".solar-reset-request.json";
const APPLICATION: &str = "net.solar.orbits";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetMarker {
    version: u32,
    application: String,
}

fn check_root(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.file_name().is_none() || path.parent().is_none() {
        return Err("Solar's data location is not safe to reset.".into());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect Solar's data folder: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Solar's data folder must be a real directory to reset it.".into());
    }
    Ok(())
}

fn read_marker(path: &Path) -> Result<Option<ResetMarker>, String> {
    let marker_path = path.join(MARKER);
    let metadata = match fs::symlink_metadata(&marker_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot inspect the Solar reset request: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Err("The Solar reset request is invalid. No files were erased.".into());
    }
    let marker: ResetMarker = serde_json::from_slice(
        &fs::read(marker_path)
            .map_err(|error| format!("Cannot read the Solar reset request: {error}"))?,
    )
    .map_err(|_| "The Solar reset request is invalid. No files were erased.".to_owned())?;
    if marker.version != 1 || marker.application != APPLICATION {
        return Err(
            "The Solar reset request does not match this application. No files were erased.".into(),
        );
    }
    Ok(Some(marker))
}

struct StagedMarker(PathBuf);

impl StagedMarker {
    fn new(root: &Path) -> Result<Self, String> {
        let path = root.join(format!(".solar-reset-{}.tmp", crate::model::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("Cannot prepare the Solar reset request: {error}"))?;
        let staged = Self(path);
        let bytes = serde_json::to_vec(&ResetMarker {
            version: 1,
            application: APPLICATION.into(),
        })
        .map_err(|error| error.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Cannot save the Solar reset request: {error}"))?;
        Ok(staged)
    }

    fn commit(&self, root: &Path) -> Result<(), String> {
        fs::rename(&self.0, root.join(MARKER))
            .map_err(|error| format!("Cannot finish preparing Solar's reset. Your local data is still present; retry or restart Solar: {error}"))
    }
}

impl Drop for StagedMarker {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Called only after the user presses Reset Solar in its confirmation modal.
/// On success the caller must restart the app without accepting new work.
pub async fn prepare_reset(engine: &Arc<Engine>, confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("Confirm Reset Solar before continuing.".into());
    }
    check_root(&engine.data_dir)?;
    if read_marker(&engine.data_dir)?.is_some() {
        return Ok(());
    }
    engine
        .resetting
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "Solar is already preparing to reset.".to_owned())?;
    let result = async {
        // Cancel running agent work before waiting for setup/connect operations
        // that may currently own native or MCP resources.
        if let Ok(active) = engine.active.lock() {
            for token in active.values() { token.cancel(); }
        }
        let _operations = tokio::time::timeout(Duration::from_secs(10), engine.reset_operations.write()).await
            .map_err(|_| "Solar is still finishing a connection or screen-sharing request. No local data was erased. Finish that request and try again.".to_owned())?;
        tokio::time::timeout(Duration::from_secs(10), engine.shutdown()).await
            .map_err(|_| "Solar could not stop all tools. No local data was erased. Wait for active work to stop and try again.".to_owned())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let empty = engine.active.lock().map_err(|_| "Task state is unavailable.")?.is_empty();
            if empty { break; }
            if Instant::now() >= deadline {
                return Err("Solar is still stopping active work. No local data was erased. Try again in a moment.".into());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        // Preflight writing before clearing credentials. The committed marker
        // exists only after the dedicated credential namespace was cleared.
        let marker = StagedMarker::new(&engine.data_dir)?;
        engine.providers.reset_credentials()?;
        marker.commit(&engine.data_dir)
    }.await;
    if result.is_err() {
        engine.resetting.store(false, Ordering::SeqCst);
    }
    result
}

/// Run before Engine::open, after the old process has released its database and
/// tools. Only entries beneath Solar's own data directory are removed.
pub fn apply_pending_reset(root: &Path) -> Result<bool, String> {
    apply_pending_reset_with(root, remove_owned_entry)
}

fn apply_pending_reset_with(
    root: &Path,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<bool, String> {
    if read_marker(root)?.is_none() {
        return Ok(false);
    }
    check_root(root)?;
    for entry in fs::read_dir(root)
        .map_err(|error| format!("Cannot open Solar's data folder for reset: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("Cannot inspect Solar's data folder for reset: {error}"))?;
        if entry.file_name() == MARKER {
            continue;
        }
        remove(&entry.path()).map_err(|error| format!("Solar's reset is incomplete. The reset request was retained so it can be retried: {error}"))?;
    }
    // Keep the marker until every owned entry is gone. Failed resets can retry
    // on launch instead of opening an incompletely cleared database.
    fs::remove_file(root.join(MARKER))
        .map_err(|error| format!("Cannot finish Solar's reset: {error}"))?;
    Ok(true)
}

fn remove_owned_entry(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTypeExt;
            if metadata.file_type().is_symlink_dir() {
                return fs::remove_dir(path);
            }
        }
        return fs::remove_file(path);
    }
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;
    use crate::model::Project;

    #[tokio::test]
    async fn reset_requires_confirmation_and_defers_erasure_until_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("solar");
        let engine = Engine::open(root.clone()).unwrap();
        assert!(prepare_reset(&engine, false).await.is_err());
        assert!(root.join("solar.sqlite").exists());
        assert!(!engine.resetting.load(Ordering::SeqCst));
        prepare_reset(&engine, true).await.unwrap();
        assert!(engine.resetting.load(Ordering::SeqCst));
        assert!(root.join("solar.sqlite").exists());
        assert!(root.join(MARKER).exists());
        drop(engine);
        assert!(apply_pending_reset(&root).unwrap());
        assert!(!apply_pending_reset(&root).unwrap());
        let fresh = Engine::open(root).unwrap();
        assert_eq!(fresh.bootstrap().unwrap().orbits.len(), 3);
        assert!(!fresh.settings().unwrap().onboarding_complete);
    }

    #[tokio::test]
    async fn reset_preserves_external_project_files() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("solar");
        let external = temporary.path().join("external-project");
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("keep.txt"), "user project").unwrap();
        let engine = Engine::open(root.clone()).unwrap();
        engine
            .save_project(Project {
                id: "project".into(),
                name: "Project".into(),
                directory: external.to_string_lossy().into_owned(),
                description: String::new(),
            })
            .unwrap();
        let workspace = engine.workspace("assistant").unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("erase.txt"), "Solar scratch work").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&external, workspace.join("outside")).unwrap();
        prepare_reset(&engine, true).await.unwrap();
        drop(engine);
        apply_pending_reset(&root).unwrap();
        assert_eq!(
            fs::read_to_string(external.join("keep.txt")).unwrap(),
            "user project"
        );
        assert!(!workspace.exists());
        assert!(!root.join("solar.sqlite").exists());
    }

    #[tokio::test]
    async fn reset_waits_for_inflight_connection_operations_before_preparing_erasure() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("solar");
        let engine = Engine::open(root.clone()).unwrap();
        let connection = engine.reset_operations.read().await;
        let resetting = tokio::spawn({
            let engine = engine.clone();
            async move { prepare_reset(&engine, true).await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !engine.resetting.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!root.join(MARKER).exists());
        assert!(!resetting.is_finished());
        drop(connection);
        resetting.await.unwrap().unwrap();
        assert!(root.join(MARKER).exists());
    }

    #[test]
    fn invalid_marker_cannot_authorize_erasure() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join(MARKER),
            r#"{"version":1,"application":"another.app"}"#,
        )
        .unwrap();
        fs::write(temporary.path().join("keep.txt"), "keep").unwrap();
        assert!(apply_pending_reset(temporary.path()).is_err());
        assert!(temporary.path().join(MARKER).exists());
        assert!(temporary.path().join("keep.txt").exists());
    }

    #[test]
    fn failed_erasure_keeps_the_reset_marker_for_retry() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join(MARKER),
            serde_json::to_vec(&ResetMarker {
                version: 1,
                application: APPLICATION.into(),
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(
            temporary.path().join("state"),
            "keep until deletion succeeds",
        )
        .unwrap();
        assert!(
            apply_pending_reset_with(temporary.path(), |_| Err(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            )))
            .is_err()
        );
        assert!(temporary.path().join(MARKER).exists());
        assert!(apply_pending_reset(temporary.path()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_root_is_never_followed() {
        let temporary = tempfile::tempdir().unwrap();
        let external = temporary.path().join("external");
        fs::create_dir_all(&external).unwrap();
        fs::write(
            external.join(MARKER),
            serde_json::to_vec(&ResetMarker {
                version: 1,
                application: APPLICATION.into(),
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(external.join("keep.txt"), "keep").unwrap();
        let link = temporary.path().join("solar");
        std::os::unix::fs::symlink(&external, &link).unwrap();
        assert!(apply_pending_reset(&link).is_err());
        assert!(external.join("keep.txt").exists());
    }
}
