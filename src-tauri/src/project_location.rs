use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectLocation {
    pub directory: String,
    pub name: String,
    pub is_git_root: bool,
}

/// Resolve from the chosen folder without changing Git state or executing hooks.
/// A worktree is its own project even when it shares another repository's objects.
pub(crate) fn resolve(directory: &str) -> Result<ProjectLocation, String> {
    let selected = std::fs::canonicalize(directory)
        .map_err(|error| format!("Select an existing project folder: {error}"))?;
    if !selected.is_dir() {
        return Err("The project path must be a folder".into());
    }
    let git_root = find_git_root(&selected);
    let directory = git_root.as_ref().unwrap_or(&selected);
    let name = directory
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or("Select a named project folder instead of the filesystem root")?
        .to_str()
        .ok_or("Project folders must have a UTF-8 name")?
        .to_owned();
    if name.len() > 200 {
        return Err("The project folder name is too long".into());
    }
    Ok(ProjectLocation {
        directory: directory
            .to_str()
            .ok_or("Project folders must have a UTF-8 path")?
            .to_owned(),
        name,
        is_git_root: git_root.is_some(),
    })
}

fn find_git_root(selected: &Path) -> Option<PathBuf> {
    let mut child = std::process::Command::new("git")
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "rev-parse",
            "--show-toplevel",
        ])
        .current_dir(selected)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) if started.elapsed() < Duration::from_secs(2) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()?
        .take(65_536)
        .read_to_end(&mut bytes)
        .ok()?;
    // Git terminates its path with a newline; preserve legal spaces in folder names.
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes))
    };
    #[cfg(not(unix))]
    let path = PathBuf::from(String::from_utf8(bytes).ok()?.trim_end_matches('\r'));
    let root = std::fs::canonicalize(path).ok()?;
    (root.is_dir() && selected.starts_with(&root)).then_some(root)
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn resolve_project_directory(
    window: tauri::WebviewWindow,
    directory: String,
) -> Result<ProjectLocation, String> {
    if window.label() != "main" {
        return Err("Choose project folders in the main Solar window".into());
    }
    tauri::async_runtime::spawn_blocking(move || resolve(&directory))
        .await
        .map_err(|_| "Could not inspect the project folder".to_owned())?
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn choose_project_directory(
    window: tauri::WebviewWindow,
    directory: Option<String>,
) -> Result<Option<ProjectLocation>, String> {
    use tauri_plugin_dialog::DialogExt;
    if window.label() != "main" {
        return Err("Choose project folders in the main Solar window".into());
    }
    let mut dialog = window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Choose a project folder");
    if let Some(path) = directory.filter(|path| Path::new(path).is_dir()) {
        dialog = dialog.set_directory(path);
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |folder| {
        let _ = sender.send(folder);
    });
    let Some(folder) = receiver
        .await
        .map_err(|_| "The folder picker closed unexpectedly".to_owned())?
    else {
        return Ok(None);
    };
    let path = folder
        .into_path()
        .map_err(|_| "Select a local folder".to_owned())?;
    let path = path
        .to_str()
        .ok_or("Project folders must have a UTF-8 path")?
        .to_owned();
    tauri::async_runtime::spawn_blocking(move || resolve(&path).map(Some))
        .await
        .map_err(|_| "Could not inspect the project folder".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(path: &Path, args: &[&str]) {
        assert!(
            std::process::Command::new("git")
                .current_dir(path)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn plain_folder_and_invalid_selection() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Plain project");
        std::fs::create_dir(&path).unwrap();
        let selected = resolve(path.to_str().unwrap()).unwrap();
        assert_eq!(selected.name, "Plain project");
        assert!(!selected.is_git_root);
        assert!(resolve(path.join("missing").to_str().unwrap()).is_err());
        std::fs::write(path.join("file"), "x").unwrap();
        assert!(resolve(path.join("file").to_str().unwrap()).is_err());
    }
    #[test]
    fn git_subfolders_use_root_and_save_derives_the_name() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("My repo");
        std::fs::create_dir_all(path.join("src/nested")).unwrap();
        git(&path, &["init", "-q"]);
        let selected = resolve(path.join("src/nested").to_str().unwrap()).unwrap();
        assert_eq!(selected.name, "My repo");
        assert!(selected.is_git_root);
        assert_eq!(Path::new(&selected.directory), path.canonicalize().unwrap());
        let engine = crate::engine::Engine::open(temp.path().join("solar")).unwrap();
        let saved = engine
            .save_project(crate::model::Project {
                id: "test".into(),
                name: "ignored custom name".into(),
                directory: path.join("src").to_string_lossy().into(),
                description: "Context".into(),
            })
            .unwrap();
        assert_eq!(saved.name, "My repo");
        assert_eq!(saved.directory, selected.directory);
    }
    #[test]
    fn worktree_and_bare_repositories_keep_the_selected_working_root() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Solar Test",
                "-c",
                "user.email=solar@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "init",
            ],
        );
        let worktree = temp.path().join("linked");
        git(
            &repo,
            &["worktree", "add", "--detach", worktree.to_str().unwrap()],
        );
        std::fs::create_dir(worktree.join("subdir")).unwrap();
        assert_eq!(
            resolve(worktree.join("subdir").to_str().unwrap())
                .unwrap()
                .name,
            "linked"
        );
        let bare = temp.path().join("bare.git");
        std::fs::create_dir(&bare).unwrap();
        git(&bare, &["init", "--bare", "-q"]);
        assert_eq!(resolve(bare.to_str().unwrap()).unwrap().name, "bare.git");
    }

    #[test]
    fn changing_project_scope_stops_existing_commands_but_context_edits_do_not() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let engine = crate::engine::Engine::open(temp.path().join("solar")).unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        engine
            .active
            .lock()
            .unwrap()
            .insert("existing-work".into(), token.clone());
        let project = engine
            .save_project(crate::model::Project {
                id: "project".into(),
                name: String::new(),
                directory: folder.to_string_lossy().into(),
                description: String::new(),
            })
            .unwrap();
        assert!(token.is_cancelled());
        let token = tokio_util::sync::CancellationToken::new();
        engine
            .active
            .lock()
            .unwrap()
            .insert("existing-work".into(), token.clone());
        let mut edited = project;
        edited.description = "Updated context".into();
        let mut edited = engine.save_project(edited).unwrap();
        assert!(!token.is_cancelled());
        let other = temp.path().join("other");
        std::fs::create_dir(&other).unwrap();
        edited.directory = other.to_string_lossy().into();
        engine.save_project(edited).unwrap();
        assert!(token.is_cancelled());
    }

    #[cfg(unix)]
    #[test]
    fn resolves_symlinks_and_preserves_spaces_in_selected_names() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Folder with trailing space ");
        std::fs::create_dir(&folder).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&folder, &alias).unwrap();
        let location = resolve(alias.to_str().unwrap()).unwrap();
        assert_eq!(location.name, "Folder with trailing space ");
        assert_eq!(Path::new(&location.directory), folder);
    }
}
