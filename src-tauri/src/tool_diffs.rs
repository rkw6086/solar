//! Read-only, per-tool working-tree snapshots. Existing edits are the baseline;
//! Git's index, HEAD, hooks, and configuration are never changed.
use crate::model::FileDiff;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime},
};
use tokio::io::AsyncReadExt;

const MAX_FILE: u64 = 1_048_576;
const MAX_FILES: usize = 4_000;
const MAX_SNAPSHOT: usize = 32 * 1_048_576;
const MAX_PATCH: usize = 65_536;
const MAX_PATCHES: usize = 100;

#[derive(PartialEq)]
enum Contents {
    Text(Vec<u8>),
    Omitted(u64, Option<SystemTime>),
}

struct File {
    label: String,
    contents: Contents,
}

#[derive(Default)]
struct Tree {
    files: BTreeMap<PathBuf, File>,
    partial: bool,
}

pub(crate) struct GitSnapshot {
    scopes: Vec<(PathBuf, PathBuf)>,
    before: Tree,
    temporary_root: PathBuf,
}

impl GitSnapshot {
    pub(crate) async fn capture(workspace: &Path, project_roots: &[PathBuf]) -> Self {
        tokio::time::timeout(
            Duration::from_secs(6),
            Self::capture_inner(workspace, project_roots),
        )
        .await
        .unwrap_or_else(|_| Self {
            scopes: Vec::new(),
            before: Tree::default(),
            temporary_root: workspace.parent().unwrap_or(workspace).join("tmp"),
        })
    }

    async fn capture_inner(workspace: &Path, project_roots: &[PathBuf]) -> Self {
        let mut scopes = BTreeSet::new();
        for path in std::iter::once(workspace).chain(project_roots.iter().map(PathBuf::as_path)) {
            let Ok(path) = tokio::fs::canonicalize(path).await else {
                continue;
            };
            let Some(bytes) = git(&path, &["rev-parse", "--show-toplevel"]).await else {
                continue;
            };
            let Ok(root) = String::from_utf8(bytes) else {
                continue;
            };
            let Ok(root) = tokio::fs::canonicalize(root.trim_end()).await else {
                continue;
            };
            if path.starts_with(&root) {
                scopes.insert((root, path));
            }
        }
        let scopes: Vec<_> = scopes.into_iter().collect();
        let before = snapshot(&scopes).await;
        Self {
            scopes,
            before,
            temporary_root: workspace.parent().unwrap_or(workspace).join("tmp"),
        }
    }

    pub(crate) async fn finish(self) -> Vec<FileDiff> {
        tokio::time::timeout(Duration::from_secs(6), self.finish_inner())
            .await
            .unwrap_or_else(|_| {
                vec![FileDiff {
                    path: "File changes".into(),
                    patch: "The diff preview timed out. Review the project for any changes.".into(),
                    truncated: true,
                }]
            })
    }

    async fn finish_inner(self) -> Vec<FileDiff> {
        if self.scopes.is_empty() {
            return Vec::new();
        }
        let after = snapshot(&self.scopes).await;
        let paths: BTreeSet<_> = self.before.files.keys().chain(after.files.keys()).collect();
        let mut diffs = Vec::new();
        let mut preview_bytes = 0;
        let mut partial = self.before.partial || after.partial;
        for path in paths {
            let old = self.before.files.get(path);
            let new = after.files.get(path);
            // A bounded/incomplete snapshot cannot prove a file was absent.
            if (old.is_none() && self.before.partial) || (new.is_none() && after.partial) {
                continue;
            }
            if old.map(|f| &f.contents) == new.map(|f| &f.contents) {
                continue;
            }
            if diffs.len() == MAX_PATCHES || preview_bytes >= 512 * 1024 {
                partial = true;
                break;
            }
            let label = &new.or(old).unwrap().label;
            let old_text = text_bytes(old);
            let new_text = text_bytes(new);
            let (patch, truncated) = match (old_text, new_text) {
                (Some(old), Some(new)) => {
                    match patch(&self.temporary_root, label, old, new).await {
                        Some(value) => value,
                        None => (
                            "This file changed. A text diff could not be generated.".into(),
                            true,
                        ),
                    }
                }
                _ => (
                    "Binary or large file changed. Text preview omitted.".into(),
                    true,
                ),
            };
            preview_bytes += patch.len();
            diffs.push(FileDiff {
                path: label.clone(),
                patch,
                truncated,
            });
        }
        if partial {
            diffs.push(FileDiff {
                path: "Additional changes".into(),
                patch: "Preview limits were reached. Some files or changes are omitted.".into(),
                truncated: true,
            });
        }
        diffs
    }
}

fn text_bytes(file: Option<&File>) -> Option<&[u8]> {
    match file.map(|file| &file.contents) {
        None => Some(&[]),
        Some(Contents::Text(bytes)) => Some(bytes),
        Some(Contents::Omitted(..)) => None,
    }
}

async fn snapshot(scopes: &[(PathBuf, PathBuf)]) -> Tree {
    let mut tree = Tree::default();
    let mut total = 0usize;
    for (repo, scope) in scopes {
        let Some(paths) = git(
            scope,
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
                "--",
                ".",
            ],
        )
        .await
        else {
            tree.partial = true;
            continue;
        };
        for relative in paths.split(|byte| *byte == 0).filter(|p| !p.is_empty()) {
            let Some(relative) = path_from_bytes(relative) else {
                tree.partial = true;
                continue;
            };
            // Git names are relative to scope, and must never escape it.
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                continue;
            }
            let path = scope.join(relative);
            if tree.files.contains_key(&path) {
                continue;
            }
            if tree.files.len() >= MAX_FILES {
                tree.partial = true;
                break;
            }
            // Do not follow file or directory symlinks into an ungranted tree.
            let Ok(canonical) = tokio::fs::canonicalize(&path).await else {
                continue;
            };
            if !canonical.starts_with(scope) {
                continue;
            }
            let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let contents = if meta.len() <= MAX_FILE
                && total.saturating_add(meta.len() as usize) <= MAX_SNAPSHOT
            {
                let Ok(file) = tokio::fs::File::open(&path).await else {
                    continue;
                };
                let mut bytes = Vec::new();
                if file
                    .take(MAX_FILE + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .is_err()
                {
                    continue;
                }
                total += bytes.len();
                if bytes.len() as u64 <= MAX_FILE
                    && !bytes.contains(&0)
                    && std::str::from_utf8(&bytes).is_ok()
                {
                    Contents::Text(bytes)
                } else {
                    Contents::Omitted(meta.len(), meta.modified().ok())
                }
            } else {
                if total.saturating_add(meta.len() as usize) > MAX_SNAPSHOT {
                    tree.partial = true;
                }
                Contents::Omitted(meta.len(), meta.modified().ok())
            };
            let relative = path.strip_prefix(repo).unwrap_or(&path);
            let label = format!(
                "{}/{}",
                repo.file_name().unwrap_or_default().to_string_lossy(),
                relative.to_string_lossy()
            );
            tree.files.insert(path, File { label, contents });
        }
    }
    tree
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Some(std::ffi::OsStr::from_bytes(bytes).into())
}
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(bytes).ok().map(PathBuf::from)
}

async fn git(directory: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut command = tokio::process::Command::new("git");
    command
        .args(["--no-pager", "--literal-pathspecs"])
        .args(args)
        .current_dir(directory)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_CONFIG_COUNT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        stdout
            .take(4 * 1_048_576)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() == 4 * 1_048_576 {
            return None;
        }
        let status = child.wait().await.ok()?;
        (status.success() || status.code() == Some(1)).then_some(bytes)
    })
    .await;
    result.ok().flatten()
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn patch(root: &Path, label: &str, before: &[u8], after: &[u8]) -> Option<(String, bool)> {
    tokio::fs::create_dir_all(root).await.ok()?;
    if tokio::fs::symlink_metadata(root)
        .await
        .ok()?
        .file_type()
        .is_symlink()
    {
        return None;
    }
    let temporary = Temporary(root.join(format!("diff-{}", uuid::Uuid::new_v4())));
    tokio::fs::create_dir(&temporary.0).await.ok()?;
    tokio::fs::write(temporary.0.join("before"), before)
        .await
        .ok()?;
    tokio::fs::write(temporary.0.join("after"), after)
        .await
        .ok()?;
    let bytes = git(
        &temporary.0,
        &[
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--color=never",
            "--",
            "before",
            "after",
        ],
    )
    .await?;
    let text = String::from_utf8(bytes).ok()?;
    let hunks = text
        .lines()
        .skip_while(|line| !line.starts_with("@@"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut preview = format!("--- a/{label}\n+++ b/{label}\n{hunks}\n");
    let truncated = preview.len() > MAX_PATCH;
    if truncated {
        let mut boundary = MAX_PATCH;
        while !preview.is_char_boundary(boundary) {
            boundary -= 1;
        }
        preview.truncate(boundary);
        preview.push_str("\n… diff preview truncated\n");
    }
    Some((preview, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn init(path: &Path) {
        assert!(git(path, &["init", "-q"]).await.is_some());
    }

    #[tokio::test]
    async fn captures_incremental_edits_additions_and_deletions_without_touching_index() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        let workspace = temp.path().join("orbit/workspace");
        tokio::fs::create_dir_all(&repo).await.unwrap();
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        init(&repo).await;
        tokio::fs::write(repo.join("edited.txt"), "already edited\n")
            .await
            .unwrap();
        tokio::fs::write(repo.join("removed.txt"), "remove me\n")
            .await
            .unwrap();
        git(&repo, &["add", "."]).await.unwrap();
        let index = tokio::fs::read(repo.join(".git/index")).await.unwrap();
        let snapshot = GitSnapshot::capture(&workspace, std::slice::from_ref(&repo)).await;
        tokio::fs::write(repo.join("edited.txt"), "tool update\n")
            .await
            .unwrap();
        tokio::fs::remove_file(repo.join("removed.txt"))
            .await
            .unwrap();
        tokio::fs::write(repo.join("new.txt"), "new contents\n")
            .await
            .unwrap();
        let diffs = snapshot.finish().await;
        assert_eq!(diffs.len(), 3);
        assert!(
            diffs
                .iter()
                .any(|d| d.patch.contains("-already edited") && d.patch.contains("+tool update"))
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.path.ends_with("new.txt") && d.patch.contains("+new contents"))
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.path.ends_with("removed.txt") && d.patch.contains("-remove me"))
        );
        assert_eq!(
            tokio::fs::read(repo.join(".git/index")).await.unwrap(),
            index
        );
    }

    #[tokio::test]
    async fn grants_cover_only_the_selected_subfolder_and_skip_ignored_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        let workspace = temp.path().join("orbit/workspace");
        tokio::fs::create_dir_all(repo.join("allowed"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        init(&repo).await;
        tokio::fs::write(repo.join(".gitignore"), "ignored.txt\n")
            .await
            .unwrap();
        let snapshot = GitSnapshot::capture(&workspace, &[repo.join("allowed")]).await;
        tokio::fs::write(repo.join("outside.txt"), "outside grant\n")
            .await
            .unwrap();
        tokio::fs::write(repo.join("allowed/ignored.txt"), "ignored\n")
            .await
            .unwrap();
        tokio::fs::write(repo.join("allowed/visible.txt"), "visible\n")
            .await
            .unwrap();
        let diffs = snapshot.finish().await;
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].path.ends_with("allowed/visible.txt"));
    }
}
