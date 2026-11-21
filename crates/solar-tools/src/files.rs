use crate::{ToolContext, ToolDefinition, ToolOutput, WorkspaceMount, definition, required_str};
use cap_std::fs::Dir;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const MAX_FILE: usize = 1_048_576;
const MAX_RESULTS: usize = 200;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        definition(
            "files_list",
            "List a directory inside this Sprite's workspace, including approved project/shared-folder links. Host paths outside the workspace are not accepted.",
            json!({"path":{"type":"string"}}),
            &[],
        ),
        definition(
            "files_read",
            "Read a UTF-8 text file through this Sprite's workspace or approved workspace links. Output is limited to 1 MiB; unapproved symlink traversal is blocked.",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
        definition(
            "files_write",
            "Atomically write a UTF-8 text file in the workspace or a writable approved link. Parent directories must exist. Read-only shares and unapproved symlinks are blocked.",
            json!({"path":{"type":"string"},"content":{"type":"string"}}),
            &["path", "content"],
        ),
        definition(
            "files_search",
            "Search text files recursively in an approved directory, returning bounded line matches. Symlink directories are skipped.",
            json!({"path":{"type":"string"},"query":{"type":"string"}}),
            &["query"],
        ),
    ]
}

pub async fn execute(
    name: &str,
    args: &Value,
    context: &ToolContext,
) -> Result<ToolOutput, String> {
    let name = name.to_owned();
    let args = args.clone();
    let context = context.clone();
    tokio::task::spawn_blocking(move || execute_sync(&name, &args, &context))
        .await
        .map_err(|error| format!("File operation stopped: {error}"))?
}

fn execute_sync(name: &str, args: &Value, context: &ToolContext) -> Result<ToolOutput, String> {
    if context.cancellation.is_cancelled() {
        return Err("File operation cancelled".into());
    }
    let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let scope = resolve_scope(path, context)?;
    let relative = &scope.relative;
    let root = &scope.root;
    match name {
        "files_list" => {
            let mut entries = Vec::new();
            let directory = open_relative_directory(root, relative)?;
            for entry in directory
                .read_dir(".")
                .map_err(|e| e.to_string())?
                .take(MAX_RESULTS)
            {
                let entry = entry.map_err(|e| e.to_string())?;
                if denied(
                    &scope.physical_root.join(relative).join(entry.file_name()),
                    context,
                ) {
                    continue;
                }
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                let logical_child = scope.logical_root.join(relative).join(entry.file_name());
                let shared = if kind.is_symlink() {
                    context.mounts.iter().find(|mount| {
                        mount
                            .path
                            .to_str()
                            .and_then(|path| workspace_path(path, context).ok())
                            .is_some_and(|path| path == logical_child)
                    })
                } else {
                    None
                };
                if let Some(mount) = shared {
                    validate_mount(context, mount)?;
                }
                entries.push(json!({"name":entry.file_name().to_string_lossy(),"directory":kind.is_dir() || shared.is_some_and(|mount| mount.target.is_dir()),"symlink":kind.is_symlink(),"shared":shared.is_some(),"writable":shared.map(|mount| mount.writable).unwrap_or(scope.writable)}));
            }
            entries.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_owned());
            Ok(ToolOutput::json(
                json!({"path":path,"entries":entries,"limit":MAX_RESULTS}),
            ))
        }
        "files_read" => {
            let file = open_relative_file(root, relative)?;
            let mut bytes = Vec::new();
            file.take((MAX_FILE + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            let truncated = bytes.len() > MAX_FILE;
            bytes.truncate(MAX_FILE);
            let content = match String::from_utf8(bytes) {
                Ok(content) => content,
                Err(error) if truncated && error.utf8_error().error_len().is_none() => {
                    let valid = error.utf8_error().valid_up_to();
                    String::from_utf8(error.into_bytes()[..valid].to_vec())
                        .map_err(|e| e.to_string())?
                }
                Err(_) => {
                    return Err("File is not UTF-8 text; use an appropriate document tool".into());
                }
            };
            Ok(ToolOutput::json(
                json!({"path":path,"content":content,"truncated":truncated}),
            ))
        }
        "files_write" => {
            if !scope.writable {
                return Err("This shared file or folder is read-only".into());
            }
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or("content must be a string")?;
            if content.len() > MAX_FILE {
                return Err("File content exceeds 1 MiB".into());
            }
            let filename = relative.file_name().ok_or("A file path is required")?;
            let parent = relative
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let directory = open_relative_directory(root, parent)?;
            if directory
                .symlink_metadata(filename)
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                return Err(
                    "Unapproved file symlinks cannot be replaced; use an approved workspace link"
                        .into(),
                );
            }
            let temporary = format!(".solar-{}.tmp", uuid::Uuid::new_v4());
            let mut options = cap_std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            let result = (|| {
                let mut file = directory.open_with(&temporary, &options)?;
                file.write_all(content.as_bytes())?;
                file.sync_all()?;
                if context.cancellation.is_cancelled() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "File write cancelled before commit",
                    ));
                }
                directory.rename(&temporary, &directory, filename)?;
                Ok::<_, std::io::Error>(())
            })();
            if result.is_err() {
                let _ = directory.remove_file(&temporary);
            }
            result.map_err(|e| e.to_string())?;
            Ok(ToolOutput::json(
                json!({"path":path,"bytes":content.len(),"written":true}),
            ))
        }
        "files_search" => {
            let query = required_str(args, "query")?;
            let mut matches = Vec::new();
            let mut scanned = 0usize;
            search(
                root,
                relative,
                query,
                context,
                &scope.physical_root,
                &scope.logical_root,
                &mut matches,
                &mut scanned,
                0,
            )?;
            Ok(ToolOutput::json(
                json!({"matches":matches,"scanned":scanned,"limit":MAX_RESULTS}),
            ))
        }
        _ => Err("Unknown file tool".into()),
    }
}

fn search(
    root: &Dir,
    path: &Path,
    query: &str,
    context: &ToolContext,
    physical_root: &Path,
    logical_root: &Path,
    matches: &mut Vec<Value>,
    scanned: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 20 || *scanned >= 2_000 || matches.len() >= MAX_RESULTS {
        return Ok(());
    }
    let directory = open_relative_directory(root, path)?;
    for entry in directory.read_dir(".").map_err(|e| e.to_string())? {
        if context.cancellation.is_cancelled() {
            return Err("Task cancelled".into());
        }
        if *scanned >= 2_000 || matches.len() >= MAX_RESULTS {
            break;
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        let child = path.join(entry.file_name());
        if denied(&physical_root.join(&child), context) {
            continue;
        }
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            search(
                root,
                &child,
                query,
                context,
                physical_root,
                logical_root,
                matches,
                scanned,
                depth + 1,
            )?;
        } else if file_type.is_file() {
            *scanned += 1;
            let Ok(file) = open_relative_file(root, &child) else {
                continue;
            };
            let mut bytes = Vec::new();
            if file.take(MAX_FILE as u64).read_to_end(&mut bytes).is_err() {
                continue;
            }
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            for (number, line) in text.lines().enumerate() {
                if line.contains(query) {
                    matches.push(json!({"path":logical_root.join(&child).to_string_lossy(),"line":number+1,"text":line.chars().take(500).collect::<String>()}));
                    if matches.len() >= MAX_RESULTS {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

/// A capability anchored at a trusted workspace or a registered link target.
/// Host paths in `allowed_roots` deliberately do not authorize direct access.
struct ScopedPath {
    root: Dir,
    relative: PathBuf,
    logical_root: PathBuf,
    physical_root: PathBuf,
    writable: bool,
}

pub(crate) fn workspace_path(input: &str, context: &ToolContext) -> Result<PathBuf, String> {
    let workspace = context
        .workspace
        .canonicalize()
        .map_err(|_| "Sprite workspace is unavailable")?;
    if workspace.parent().is_none() {
        return Err("A filesystem root cannot be a Sprite workspace".into());
    }
    let input = Path::new(input);
    if input
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("Workspace paths cannot contain parent traversal".into());
    }
    let absolute = if input.is_absolute() {
        input.to_owned()
    } else {
        workspace.join(input)
    };
    // Accept the trusted spelling of this workspace as well as its canonical
    // spelling (notably /var and /private/var on macOS).
    let absolute = if let Ok(tail) = absolute.strip_prefix(&context.workspace) {
        workspace.join(tail)
    } else {
        absolute
    };
    let absolute = absolute
        .components()
        .filter(|part| !matches!(part, Component::CurDir))
        .collect::<PathBuf>();
    if !absolute.starts_with(&workspace) {
        return Err("Use a path inside the Sprite workspace, including its approved links".into());
    }
    Ok(absolute)
}

pub(crate) fn denied(path: &Path, context: &ToolContext) -> bool {
    context.denied_roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        path.starts_with(root)
    })
}

/// Validate the on-disk alias without trusting or following an unlisted link.
pub(crate) fn validate_mount(context: &ToolContext, mount: &WorkspaceMount) -> Result<(), String> {
    let path = workspace_path(
        mount
            .path
            .to_str()
            .ok_or("Workspace links require UTF-8 paths")?,
        context,
    )?;
    if path
        == context
            .workspace
            .canonicalize()
            .map_err(|_| "Sprite workspace is unavailable")?
        || !mount.target.is_absolute()
    {
        return Err("Invalid workspace link boundary".into());
    }
    if ["/proc", "/sys", "/dev", "/run", "/private/var/run"]
        .iter()
        .any(|root| mount.target.starts_with(root))
    {
        return Err("System process, device and service-socket trees cannot be shared into a Sprite workspace".into());
    }
    let workspace = context
        .workspace
        .canonicalize()
        .map_err(|_| "Sprite workspace is unavailable")?;
    let relative = path
        .strip_prefix(&workspace)
        .map_err(|_| "Workspace link is outside its home")?;
    let parent = relative.parent().unwrap_or(Path::new("."));
    let directory = open_relative_directory(&open_absolute_directory(&workspace)?, parent)?;
    let name = relative.file_name().ok_or("Workspace link has no name")?;
    let actual = directory.read_link_contents(name).map_err(
        |_| "An approved workspace link was removed or replaced; refresh sharing settings",
    )?;
    if actual != mount.target {
        return Err("An approved workspace link was retargeted; refresh sharing settings".into());
    }
    // Walk the stored canonical path without following mutable ancestor links.
    let parent = mount
        .target
        .parent()
        .ok_or("A filesystem root cannot be shared")?;
    let target_parent = open_absolute_directory(parent)?;
    let name = mount
        .target
        .file_name()
        .ok_or("A filesystem root cannot be shared")?;
    let metadata = target_parent
        .symlink_metadata(name)
        .map_err(|_| "Shared target is unavailable")?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err(
            "A shared target was replaced by a link or unsupported file; refresh sharing settings"
                .into(),
        );
    }
    Ok(())
}

fn resolve_scope(input: &str, context: &ToolContext) -> Result<ScopedPath, String> {
    let absolute = workspace_path(input, context)?;
    let workspace = context
        .workspace
        .canonicalize()
        .map_err(|_| "Sprite workspace is unavailable")?;
    let mounts = context
        .mounts
        .iter()
        .map(|mount| {
            Ok((
                mount,
                workspace_path(
                    mount
                        .path
                        .to_str()
                        .ok_or("Workspace links require UTF-8 paths")?,
                    context,
                )?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let selected = mounts
        .iter()
        .filter(|(_, path)| absolute.starts_with(path))
        .max_by_key(|(_, path)| path.components().count());
    let (physical_root, logical_root, relative, mut writable) =
        if let Some((mount, mount_path)) = selected {
            validate_mount(context, mount)?;
            let tail = absolute
                .strip_prefix(mount_path)
                .map_err(|_| "Invalid workspace link")?;
            if mount.target.is_file() {
                if !tail.as_os_str().is_empty() {
                    return Err("A shared file cannot contain child paths".into());
                }
                (
                    mount.target.parent().unwrap().to_owned(),
                    mount_path.parent().unwrap().to_owned(),
                    PathBuf::from(mount.target.file_name().unwrap()),
                    mount.writable,
                )
            } else {
                (
                    mount.target.clone(),
                    mount_path.clone(),
                    if tail.as_os_str().is_empty() {
                        PathBuf::from(".")
                    } else {
                        tail.to_owned()
                    },
                    mount.writable,
                )
            }
        } else {
            let relative = absolute
                .strip_prefix(&workspace)
                .map_err(|_| "Path is outside the Sprite workspace")?;
            (
                workspace.clone(),
                workspace,
                if relative.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    relative.to_owned()
                },
                true,
            )
        };
    if denied(&physical_root.join(&relative), context) {
        return Err("This path is in a project the Sprite cannot access".into());
    }
    // A second writable alias must not reopen a read-only shared target.
    writable &= !context
        .mounts
        .iter()
        .any(|mount| !mount.writable && physical_root.join(&relative).starts_with(&mount.target));
    Ok(ScopedPath {
        root: open_absolute_directory(&physical_root)?,
        relative,
        logical_root,
        physical_root,
        writable,
    })
}

pub(crate) fn working_directory(input: &str, context: &ToolContext) -> Result<PathBuf, String> {
    let scope = resolve_scope(input, context)?;
    open_relative_directory(&scope.root, &scope.relative)?;
    workspace_path(input, context)
}

pub(crate) fn workspace_executable(input: &str, context: &ToolContext) -> Result<(), String> {
    let scope = resolve_scope(input, context)?;
    let file = open_relative_file(&scope.root, &scope.relative)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file
            .into_std()
            .metadata()
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err("The approved workspace shell is not executable".into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err("Workspace shell execution is unavailable on this platform".into())
    }
}

/// Every component is opened relative to an already-held capability with
/// O_NOFOLLOW. A concurrent symlink replacement therefore fails rather than
/// jumping to another host path, including a denied child of an allowed tree.
fn open_relative_directory(root: &Dir, path: &Path) -> Result<Dir, String> {
    let mut directory = root.try_clone().map_err(|e| e.to_string())?;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => directory = open_child_directory(&directory, name)?,
            _ => return Err("Directory traversal escaped the workspace capability".into()),
        }
    }
    Ok(directory)
}

fn open_absolute_directory(path: &Path) -> Result<Dir, String> {
    if !path.is_absolute() {
        return Err("A trusted directory must be absolute".into());
    }
    let root = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    open_relative_directory(
        &root,
        path.strip_prefix("/")
            .map_err(|_| "Invalid absolute directory")?,
    )
}

#[cfg(unix)]
fn open_child_directory(root: &Dir, name: &std::ffi::OsStr) -> Result<Dir, String> {
    Ok(Dir::from_std_file(open_child(root, name, true)?))
}

#[cfg(unix)]
fn open_child(
    root: &Dir,
    name: &std::ffi::OsStr,
    directory: bool,
) -> Result<std::fs::File, String> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };
    let name =
        std::ffi::CString::new(name.as_bytes()).map_err(|_| "File path contains a null byte")?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: the directory capability remains alive and name is one normal
    // component; the resulting owned descriptor is closed by File on drop.
    let descriptor = unsafe { libc::openat(root.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(format!(
            "Cannot open workspace path without following an unapproved link: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err("Workspace path is not a regular file or directory".into());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_child_directory(_: &Dir, _: &std::ffi::OsStr) -> Result<Dir, String> {
    Err("Secure workspace file traversal is unavailable on this platform".into())
}

fn open_relative_file(root: &Dir, path: &Path) -> Result<cap_std::fs::File, String> {
    let name = path.file_name().ok_or("A regular file path is required")?;
    let parent = open_relative_directory(root, path.parent().unwrap_or(Path::new(".")))?;
    #[cfg(unix)]
    {
        Ok(cap_std::fs::File::from_std(open_child(
            &parent, name, false,
        )?))
    }
    #[cfg(not(unix))]
    {
        let _ = (name, parent);
        Err("Secure workspace file traversal is unavailable on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(root: &Path) -> ToolContext {
        ToolContext {
            orbit_id: "test".into(),
            workspace: root.into(),
            data_directory: root.into(),
            allowed_roots: vec![],
            mounts: vec![],
            denied_roots: vec![],
            allow_host_execution: false,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        }
    }
    #[tokio::test]
    async fn file_write_read_and_traversal() {
        let root = tempfile::tempdir().unwrap();
        let ctx = context(root.path());
        execute(
            "files_write",
            &json!({"path":"note.txt","content":"hello"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(
            execute("files_read", &json!({"path":"note.txt"}), &ctx)
                .await
                .unwrap()
                .content
                .contains("hello")
        );
        assert!(
            execute("files_read", &json!({"path":"../secret.txt"}), &ctx)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_cancelled_file_write_keeps_existing_content_and_creates_nothing() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("keep"), "original").unwrap();
        let ctx = context(root.path());
        ctx.cancellation.cancel();
        for path in ["keep", "new"] {
            assert!(
                execute(
                    "files_write",
                    &json!({"path":path,"content":"changed"}),
                    &ctx
                )
                .await
                .unwrap_err()
                .contains("cancelled")
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.path().join("keep")).unwrap(),
            "original"
        );
        assert!(!root.path().join("new").exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_cannot_escape_approved_directory() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("secret"), "private").unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("escape")).unwrap();
        let ctx = context(root.path());
        assert!(
            execute("files_read", &json!({"path":"escape/secret"}), &ctx)
                .await
                .is_err()
        );
        assert!(
            execute(
                "files_write",
                &json!({"path":"escape/secret","content":"changed"}),
                &ctx
            )
            .await
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(other.path().join("secret")).unwrap(),
            "private"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn approved_directory_links_are_live_but_host_paths_and_retargeting_fail() {
        let workspace = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(shared.path().join("note"), "first").unwrap();
        std::fs::write(other.path().join("note"), "private").unwrap();
        let link = workspace.path().join("shared");
        std::os::unix::fs::symlink(shared.path(), &link).unwrap();
        let mut ctx = context(workspace.path());
        ctx.allowed_roots = vec![shared.path().into(), other.path().into()];
        ctx.mounts.push(WorkspaceMount {
            path: link.clone(),
            target: shared.path().canonicalize().unwrap(),
            writable: true,
        });
        let listing: Value = serde_json::from_str(
            &execute("files_list", &json!({}), &ctx)
                .await
                .unwrap()
                .content,
        )
        .unwrap();
        let entry = listing["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "shared")
            .unwrap();
        assert_eq!(entry["directory"], true);
        assert_eq!(entry["shared"], true);
        assert!(
            execute("files_read", &json!({"path":"shared/note"}), &ctx)
                .await
                .unwrap()
                .content
                .contains("first")
        );
        assert!(
            execute(
                "files_read",
                &json!({"path":shared.path().join("note")}),
                &ctx
            )
            .await
            .is_err()
        );
        execute(
            "files_write",
            &json!({"path":"shared/note","content":"updated"}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(shared.path().join("note")).unwrap(),
            "updated"
        );
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(other.path(), &link).unwrap();
        assert!(
            execute("files_read", &json!({"path":"shared/note"}), &ctx)
                .await
                .unwrap_err()
                .contains("retargeted")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shared_files_preserve_readonly_and_do_not_grant_the_parent() {
        let workspace = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join("AGENTS.md");
        std::fs::write(&source, "shared guidance").unwrap();
        std::fs::write(home.path().join("private"), "secret").unwrap();
        let link = workspace.path().join("AGENTS.md");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        let mut ctx = context(workspace.path());
        ctx.mounts.push(WorkspaceMount {
            path: link,
            target: source.canonicalize().unwrap(),
            writable: false,
        });
        assert!(
            execute("files_read", &json!({"path":"AGENTS.md"}), &ctx)
                .await
                .unwrap()
                .content
                .contains("shared guidance")
        );
        assert!(
            execute(
                "files_write",
                &json!({"path":"AGENTS.md","content":"modified"}),
                &ctx
            )
            .await
            .unwrap_err()
            .contains("read-only")
        );
        assert!(
            execute("files_read", &json!({"path":"AGENTS.md/../private"}), &ctx)
                .await
                .is_err()
        );
        assert!(
            execute(
                "files_read",
                &json!({"path":home.path().join("private")}),
                &ctx
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(source).unwrap(), "shared guidance");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nested_links_and_denied_projects_cannot_escape_a_shared_parent() {
        let workspace = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        std::fs::write(private.path().join("secret"), "needle secret").unwrap();
        std::fs::write(shared.path().join("visible"), "needle visible").unwrap();
        std::fs::create_dir(shared.path().join("unassigned")).unwrap();
        std::fs::write(shared.path().join("unassigned/secret"), "needle denied").unwrap();
        std::os::unix::fs::symlink(private.path(), shared.path().join("escape")).unwrap();
        std::os::unix::fs::symlink(shared.path().join("visible"), shared.path().join("alias"))
            .unwrap();
        std::os::unix::fs::symlink(shared.path(), workspace.path().join("shared")).unwrap();
        let mut ctx = context(workspace.path());
        ctx.mounts.push(WorkspaceMount {
            path: workspace.path().join("shared"),
            target: shared.path().canonicalize().unwrap(),
            writable: true,
        });
        ctx.denied_roots.push(shared.path().join("unassigned"));
        for path in [
            "shared/escape/secret",
            "shared/alias",
            "shared/unassigned/secret",
        ] {
            assert!(
                execute("files_read", &json!({"path":path}), &ctx)
                    .await
                    .is_err(),
                "{path}"
            );
        }
        let result = execute(
            "files_search",
            &json!({"path":"shared","query":"needle"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(result.content.contains("needle visible"));
        assert!(
            !result.content.contains("needle denied") && !result.content.contains("needle secret")
        );
        let listed = execute("files_list", &json!({"path":"shared"}), &ctx)
            .await
            .unwrap();
        assert!(!listed.content.contains("unassigned"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn readonly_target_cannot_be_reopened_through_a_writable_parent_alias() {
        let workspace = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        std::fs::create_dir(shared.path().join("reference")).unwrap();
        std::fs::write(shared.path().join("reference/guide"), "original").unwrap();
        std::os::unix::fs::symlink(shared.path(), workspace.path().join("all")).unwrap();
        std::os::unix::fs::symlink(
            shared.path().join("reference"),
            workspace.path().join("reference"),
        )
        .unwrap();
        let mut ctx = context(workspace.path());
        ctx.mounts = vec![
            WorkspaceMount {
                path: workspace.path().join("all"),
                target: shared.path().canonicalize().unwrap(),
                writable: true,
            },
            WorkspaceMount {
                path: workspace.path().join("reference"),
                target: shared.path().join("reference").canonicalize().unwrap(),
                writable: false,
            },
        ];
        assert!(
            execute(
                "files_write",
                &json!({"path":"all/reference/guide","content":"changed"}),
                &ctx
            )
            .await
            .unwrap_err()
            .contains("read-only")
        );
        assert_eq!(
            std::fs::read_to_string(shared.path().join("reference/guide")).unwrap(),
            "original"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shared_file_parent_retarget_does_not_change_its_capability() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let shared = root.path().join("shared");
        let other = root.path().join("other");
        for path in [&workspace, &shared, &other] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(shared.join("guide"), "approved").unwrap();
        std::fs::write(other.join("guide"), "private").unwrap();
        let mut ctx = context(&workspace);
        let target = shared.join("guide");
        std::os::unix::fs::symlink(&target, workspace.join("guide")).unwrap();
        ctx.mounts.push(WorkspaceMount {
            path: workspace.join("guide"),
            target,
            writable: true,
        });
        std::fs::rename(&shared, root.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(&other, &shared).unwrap();
        assert!(
            execute("files_read", &json!({"path":"guide"}), &ctx)
                .await
                .is_err()
        );
        assert!(
            execute(
                "files_write",
                &json!({"path":"guide","content":"changed"}),
                &ctx
            )
            .await
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(other.join("guide")).unwrap(),
            "private"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn trusted_workspace_spelling_is_normalized_before_matching_mounts() {
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("actual");
        let alias = root.path().join("alias");
        let shared = root.path().join("shared");
        std::fs::create_dir_all(actual.join("workspace")).unwrap();
        std::fs::create_dir(&shared).unwrap();
        std::fs::write(shared.join("file"), "live").unwrap();
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        let workspace = alias.join("workspace");
        std::os::unix::fs::symlink(&shared, workspace.join("shared")).unwrap();
        let mut ctx = context(&workspace);
        ctx.mounts.push(WorkspaceMount {
            path: workspace.join("shared"),
            target: shared.canonicalize().unwrap(),
            writable: false,
        });
        for path in [
            workspace.join("shared/file"),
            actual.join("workspace/shared/file"),
        ] {
            assert!(
                execute("files_read", &json!({"path":path}), &ctx)
                    .await
                    .unwrap()
                    .content
                    .contains("live")
            );
        }
        assert!(
            execute("files_read", &json!({"path":shared.join("file")}), &ctx)
                .await
                .is_err()
        );
    }
}
