//! Solar keeps all managed application data beneath the user's ~/.solar folder.

use std::fs;
use std::path::{Path, PathBuf};

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(format!(
                "Solar's data location must be a real directory, not a file or symbolic link: {}",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::create_dir(path) {
                Ok(()) => {}
                // Another launch may have created it after the first check.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(format!("Cannot create Solar's data folder: {error}")),
            }
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("Cannot inspect Solar's data folder: {error}"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("Solar's data folder was replaced by a file or symbolic link.".into());
            }
        }
        Err(error) => return Err(format!("Cannot inspect Solar's data folder: {error}")),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Cannot protect Solar's data folder: {error}"))?;
    }
    Ok(())
}

/// Resolve and prepare ~/.solar. This does not inspect or migrate an old profile.
pub(crate) fn prepare(home: &Path) -> Result<PathBuf, String> {
    if !home.is_absolute() {
        return Err("Solar needs an absolute home directory.".into());
    }
    let metadata = fs::metadata(home)
        .map_err(|error| format!("Cannot inspect the home directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("Solar's home location is not a directory.".into());
    }
    let root = home.join(".solar");
    ensure_private_directory(&root)?;
    for name in ["skills", "mcp", "projects"] {
        ensure_private_directory(&root.join(name))?;
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_the_requested_layout_and_can_be_reopened() {
        let home = tempfile::tempdir().unwrap();
        let root = prepare(home.path()).unwrap();
        assert_eq!(root, home.path().join(".solar"));
        for name in ["skills", "mcp", "projects"] {
            assert!(root.join(name).is_dir());
        }
        fs::write(root.join("keep.txt"), "existing data").unwrap();
        assert_eq!(prepare(home.path()).unwrap(), root);
        assert_eq!(
            fs::read_to_string(root.join("keep.txt")).unwrap(),
            "existing data"
        );
    }

    #[test]
    fn invalid_home_is_rejected() {
        assert!(prepare(Path::new("relative/home")).is_err());
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("not-a-directory");
        fs::write(&file, "file").unwrap();
        assert!(prepare(&file).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn root_and_managed_subdirectory_links_are_not_followed() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = home.path().join(".solar");
        std::os::unix::fs::symlink(outside.path(), &root).unwrap();
        assert!(prepare(home.path()).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        fs::remove_file(&root).unwrap();
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("skills")).unwrap();
        assert!(prepare(home.path()).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn managed_directories_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let root = prepare(home.path()).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        prepare(home.path()).unwrap();
        for directory in [
            root.clone(),
            root.join("skills"),
            root.join("mcp"),
            root.join("projects"),
        ] {
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
}
