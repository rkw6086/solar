//! OS-enforced exclusions for registered projects outside a task's grants.
//! This deliberately leaves other host access unchanged. It is not a general
//! purpose sandbox for hostile code or for separately running services.
use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(crate) fn command(
    shell: &Path,
    arguments: &[OsString],
    denied_roots: &[PathBuf],
) -> Result<tokio::process::Command, String> {
    let roots = canonical_roots(denied_roots)?;
    if roots.is_empty() {
        let mut command = tokio::process::Command::new(shell);
        command.args(arguments);
        return Ok(command);
    }
    scoped_command(shell, arguments, &roots)
}

fn canonical_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut canonical = BTreeSet::new();
    for root in roots {
        let root = std::fs::canonicalize(root).map_err(|_| {
            "Cannot protect an unassigned project folder. Check its location before running shell commands.".to_owned()
        })?;
        if !root.is_dir() || root.parent().is_none() {
            return Err(
                "An unassigned project location cannot be safely excluded from shell access."
                    .into(),
            );
        }
        canonical.insert(root);
    }
    // A parent exclusion already covers every child; do not create nested mounts.
    Ok(canonical
        .iter()
        .filter(|root| {
            !canonical
                .iter()
                .any(|parent| parent != *root && root.starts_with(parent))
        })
        .cloned()
        .collect())
}

#[cfg(target_os = "linux")]
fn scoped_command(
    shell: &Path,
    arguments: &[OsString],
    roots: &[PathBuf],
) -> Result<tokio::process::Command, String> {
    let executable = ["/usr/bin/bwrap", "/bin/bwrap"].into_iter().find(|path| Path::new(path).is_file())
        .ok_or("Shell project isolation requires Bubblewrap (bwrap). Install it before running commands with unassigned projects.")?;
    let mut command = tokio::process::Command::new(executable);
    command.args([
        "--bind",
        "/",
        "/",
        "--unshare-all",
        "--unshare-user",
        "--share-net",
        "--disable-userns",
        "--cap-drop",
        "ALL",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--new-session",
        "--die-with-parent",
    ]);
    for root in roots {
        // Both the root and all symlinks resolving to it see an empty, read-only
        // filesystem. The process cannot unmount it or create another userns.
        command
            .arg("--tmpfs")
            .arg(root)
            .arg("--remount-ro")
            .arg(root);
    }
    command.arg("--").arg(shell).args(arguments);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn scoped_command(
    shell: &Path,
    arguments: &[OsString],
    roots: &[PathBuf],
) -> Result<tokio::process::Command, String> {
    let executable = Path::new("/usr/bin/sandbox-exec");
    if !executable.is_file() {
        return Err(
            "Shell project isolation is unavailable on this Mac. Commands were not started.".into(),
        );
    }
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("-p")
        .arg(macos_profile(roots)?)
        .arg(shell)
        .args(arguments);
    Ok(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn scoped_command(
    _: &Path,
    _: &[OsString],
    _: &[PathBuf],
) -> Result<tokio::process::Command, String> {
    Err("Shell project isolation is not supported on this operating system. Commands were not started.".into())
}

#[cfg(any(target_os = "macos", test))]
fn macos_profile(roots: &[PathBuf]) -> Result<String, String> {
    let mut profile = "(version 1)\n(allow default)\n".to_owned();
    let mut ancestors = BTreeSet::new();
    for root in roots {
        let path = profile_path(root)?;
        profile.push_str(&format!("(deny file-read* file-write* (subpath {path}))\n"));
        // Prevent moving an excluded directory (or its ancestors) to a new
        // pathname and thereby sidestepping the exclusion.
        for ancestor in root.ancestors().take_while(|path| path.parent().is_some()) {
            ancestors.insert(ancestor.to_owned());
        }
    }
    for ancestor in ancestors {
        profile.push_str(&format!(
            "(deny file-write-unlink (literal {}))\n",
            profile_path(&ancestor)?
        ));
    }
    Ok(profile)
}

#[cfg(any(target_os = "macos", test))]
fn profile_path(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or("Project isolation requires a UTF-8 folder path on macOS.")?;
    if text.chars().any(char::is_control) {
        return Err(
            "Project isolation cannot use folder names containing control characters.".into(),
        );
    }
    serde_json::to_string(text).map_err(|_| "Cannot encode project folder for isolation.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_escapes_paths_and_protects_ancestor_renames() {
        let profile = macos_profile(&[PathBuf::from("/Users/safe/project\"name")]).unwrap();
        assert!(
            profile.contains(
                "(deny file-read* file-write* (subpath \"/Users/safe/project\\\"name\"))"
            )
        );
        assert!(profile.contains("(deny file-write-unlink (literal \"/Users/safe\"))"));
        assert!(!profile.contains("(literal \"/\")"));
        assert!(profile_path(Path::new("/tmp/line\nbreak")).is_err());
    }

    #[test]
    fn missing_exclusions_fail_closed_and_nested_roots_collapse() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("child")).unwrap();
        let roots = canonical_roots(&[temp.path().into(), temp.path().join("child")]).unwrap();
        assert_eq!(roots, vec![temp.path().canonicalize().unwrap()]);
        assert!(
            command(
                Path::new("/bin/sh"),
                &["-c".into(), "true".into()],
                &[temp.path().join("missing")]
            )
            .is_err()
        );
    }
}
