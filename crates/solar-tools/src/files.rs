use crate::{ToolContext, ToolDefinition, ToolOutput, definition, required_str};
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
            "List a directory inside this Orbit's workspace or an explicitly granted directory.",
            json!({"path":{"type":"string"}}),
            &[],
        ),
        definition(
            "files_read",
            "Read a UTF-8 text file in an approved directory. Output is limited to 1 MiB.",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
        definition(
            "files_write",
            "Atomically write a UTF-8 text file in an approved directory. Parent directories must already exist.",
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
    let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let (root, relative) = resolve_scope(path, context)?;
    let root =
        Dir::open_ambient_dir(&root, cap_std::ambient_authority()).map_err(|e| e.to_string())?;
    match name {
        "files_list" => {
            let mut entries = Vec::new();
            for entry in root
                .read_dir(&relative)
                .map_err(|e| e.to_string())?
                .take(MAX_RESULTS)
            {
                let entry = entry.map_err(|e| e.to_string())?;
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                entries.push(json!({"name":entry.file_name().to_string_lossy(),"directory":kind.is_dir(),"symlink":kind.is_symlink()}));
            }
            entries.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_owned());
            Ok(ToolOutput::json(
                json!({"path":path,"entries":entries,"limit":MAX_RESULTS}),
            ))
        }
        "files_read" => {
            let file = root.open(&relative).map_err(|e| e.to_string())?;
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
            let directory = root.open_dir(parent).map_err(|e| e.to_string())?;
            let temporary = format!(".solar-{}.tmp", uuid::Uuid::new_v4());
            let mut options = cap_std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            let result = (|| {
                let mut file = directory.open_with(&temporary, &options)?;
                file.write_all(content.as_bytes())?;
                file.sync_all()?;
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
                &root,
                &relative,
                query,
                context,
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
    matches: &mut Vec<Value>,
    scanned: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 20 || *scanned >= 2_000 || matches.len() >= MAX_RESULTS {
        return Ok(());
    }
    for entry in root.read_dir(path).map_err(|e| e.to_string())? {
        if context.cancellation.is_cancelled() {
            return Err("Task cancelled".into());
        }
        if *scanned >= 2_000 || matches.len() >= MAX_RESULTS {
            break;
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        let child = path.join(entry.file_name());
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            search(root, &child, query, context, matches, scanned, depth + 1)?;
        } else if file_type.is_file() {
            *scanned += 1;
            let Ok(file) = root.open(&child) else {
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
                    matches.push(json!({"path":child.to_string_lossy(),"line":number+1,"text":line.chars().take(500).collect::<String>()}));
                    if matches.len() >= MAX_RESULTS {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Choose an explicitly granted root, then retain a relative capability path.
/// cap-std performs the actual traversal and rejects escapes, including races.
fn resolve_scope(input: &str, context: &ToolContext) -> Result<(PathBuf, PathBuf), String> {
    let input = Path::new(input);
    let absolute = if input.is_absolute() {
        input.to_path_buf()
    } else {
        std::fs::canonicalize(&context.workspace)
            .map_err(|e| e.to_string())?
            .join(input)
    };
    let absolute = normalize(&absolute)?;
    let mut roots = vec![context.workspace.clone()];
    roots.extend(context.allowed_roots.iter().cloned());
    roots.sort_by_key(|root| std::cmp::Reverse(root.components().count()));
    for root in roots {
        let Ok(root) = std::fs::canonicalize(root) else {
            continue;
        };
        if let Ok(relative) = absolute.strip_prefix(&root) {
            let relative = if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative.to_path_buf()
            };
            return Ok((root, relative));
        }
    }
    Err("Path is outside this Orbit's approved directories".into())
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if !result.pop() {
                    return Err("Path escapes filesystem root".into());
                }
            }
            Component::CurDir => {}
            component => result.push(component.as_os_str()),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(root: &Path) -> ToolContext {
        ToolContext {
            orbit_id: "test".into(),
            workspace: root.into(),
            allowed_roots: vec![],
            allow_host_execution: false,
            screen_share_allowed: false,
            cancellation: Default::default(),
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
}
