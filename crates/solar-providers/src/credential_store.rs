//! Private, persistent credential files owned by Solar. No platform credential
//! service is involved. The containing directory is the confidentiality boundary;
//! contents are not encrypted with a key stored alongside them.
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};

const MAX_SECRET_BYTES: usize = 256 * 1024;

pub(crate) struct CredentialStore {
    directory: Dir,
}

impl CredentialStore {
    pub(crate) fn open(data_directory: &Path) -> Result<Self, String> {
        if !data_directory.is_absolute() {
            return Err("Solar credential storage needs an absolute data directory.".into());
        }
        let metadata = std::fs::symlink_metadata(data_directory)
            .map_err(|_| "Solar's data directory is unavailable.")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Solar's data directory must be a real directory.".into());
        }
        #[cfg(unix)]
        let root = {
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = std::fs::OpenOptions::new();
            options
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
            Dir::from_std_file(
                options
                    .open(data_directory)
                    .map_err(|_| "Cannot open Solar's private data directory.")?,
            )
        };
        #[cfg(not(unix))]
        let root = Dir::open_ambient_dir(data_directory, cap_std::ambient_authority())
            .map_err(|_| "Cannot open Solar's private data directory.")?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            root.set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect Solar's private data directory.")?;
        }
        match root.create_dir("credentials") {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("Cannot create Solar's credential directory.".into()),
        }
        let metadata = root
            .symlink_metadata("credentials")
            .map_err(|_| "Cannot inspect Solar's credential directory.")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Solar's credential directory must be a real directory.".into());
        }
        #[cfg(unix)]
        let directory = {
            use cap_std::fs::OpenOptionsExt;
            let mut options = OpenOptions::new();
            options
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
            Dir::from_std_file(
                root.open_with("credentials", &options)
                    .map_err(|_| "Cannot open Solar's credential directory.")?
                    .into_std(),
            )
        };
        #[cfg(not(unix))]
        let directory = root
            .open_dir("credentials")
            .map_err(|_| "Cannot open Solar's credential directory.")?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            directory
                .set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect Solar's credential directory.")?;
        }
        Ok(Self { directory })
    }

    pub(crate) fn load(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let filename = filename(key);
        match self.directory.symlink_metadata(&filename) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {},
            Ok(_) => return Err("A Solar credential file is not a regular file. Reconnect this connection after repairing its storage.".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Cannot inspect Solar's saved credentials.".into()),
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = self
            .directory
            .open_with(&filename, &options)
            .map_err(|_| "Cannot read Solar's saved credentials.")?;
        let metadata = file
            .metadata()
            .map_err(|_| "Cannot inspect Solar's saved credentials.")?;
        if !metadata.is_file() || metadata.len() > MAX_SECRET_BYTES as u64 {
            return Err(
                "Solar's saved credential file is invalid. Reconnect this connection.".into(),
            );
        }
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            file.set_permissions(cap_std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Cannot protect Solar's saved credentials.")?;
        }
        let mut bytes = Vec::new();
        file.take((MAX_SECRET_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read Solar's saved credentials.")?;
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(
                "Solar's saved credential file is too large. Reconnect this connection.".into(),
            );
        }
        Ok(Some(bytes))
    }

    pub(crate) fn save(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_SECRET_BYTES {
            return Err("Credential exceeds Solar's storage limit.".into());
        }
        let filename = filename(key);
        self.check_regular_or_missing(&filename)?;
        let temporary = format!(".credential-{}.tmp", uuid::Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = self
                .directory
                .open_with(&temporary, &options)
                .map_err(|_| "Cannot create Solar's private credential file.")?;
            #[cfg(unix)]
            {
                use cap_std::fs::PermissionsExt;
                file.set_permissions(cap_std::fs::Permissions::from_mode(0o600))
                    .map_err(|_| "Cannot protect Solar's private credential file.")?;
            }
            file.write_all(bytes)
                .map_err(|_| "Cannot save Solar's credentials.")?;
            file.sync_all()
                .map_err(|_| "Cannot save Solar's credentials.")?;
            // A changed symlink is never followed or overwritten during rotation.
            self.check_regular_or_missing(&filename)?;
            self.directory
                .rename(&temporary, &self.directory, &filename)
                .map_err(|_| "Cannot finish saving Solar's credentials.")?;
            self.sync_directory()
        })();
        if result.is_err() {
            let _ = self.directory.remove_file(&temporary);
        }
        result
    }

    fn check_regular_or_missing(&self, filename: &str) -> Result<(), String> {
        match self.directory.symlink_metadata(filename) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
            Ok(_) => Err("A Solar credential path is not a regular file.".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Cannot inspect Solar's credential storage.".into()),
        }
    }

    pub(crate) fn delete(&self, key: &str) -> Result<(), String> {
        self.remove_owned_file(&filename(key))?;
        self.sync_directory()
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        for entry in self
            .directory
            .entries()
            .map_err(|_| "Cannot read Solar's credential directory.")?
        {
            let entry = entry.map_err(|_| "Cannot inspect Solar's credential directory.")?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if owned_filename(name) {
                self.remove_owned_file(name)?;
            }
        }
        self.sync_directory()
    }

    fn remove_owned_file(&self, name: &str) -> Result<(), String> {
        // remove_file unlinks the leaf itself, including a swapped symbolic link;
        // it never traverses that link into another application's data.
        match self.directory.remove_file(name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Cannot remove Solar's saved credentials.".into()),
        }
    }

    fn sync_directory(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            let directory = self
                .directory
                .try_clone()
                .map_err(|_| "Cannot finish updating Solar's credential storage.")?;
            directory_sync_result(directory.into_std_file().sync_all())?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn directory_sync_result(result: std::io::Result<()>) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        // File content was synced before atomic replacement. Some filesystems
        // do not support fsync on directory descriptors; that must not turn a
        // committed credential update into a reported failure. Real I/O errors
        // remain failures, rather than claiming stronger durability than exists.
        Err(error) if matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOTSUP)) => Ok(()),
        Err(_) => Err("Cannot finish updating Solar's credential storage.".into()),
    }
}

fn filename(key: &str) -> String {
    format!("{:x}.json", Sha256::digest(key.as_bytes()))
}
fn owned_filename(name: &str) -> bool {
    name.strip_suffix(".json")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        || (name.starts_with(".credential-") && name.ends_with(".tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn directory_sync_ignores_only_unsupported_operations() {
        assert!(directory_sync_result(Ok(())).is_ok());
        assert!(
            directory_sync_result(Err(std::io::Error::from_raw_os_error(libc::EINVAL))).is_ok()
        );
        assert!(
            directory_sync_result(Err(std::io::Error::from_raw_os_error(libc::ENOTSUP))).is_ok()
        );
        assert!(directory_sync_result(Err(std::io::Error::from_raw_os_error(libc::EIO))).is_err());
        assert!(
            directory_sync_result(Err(std::io::Error::from_raw_os_error(libc::EACCES))).is_err()
        );
    }
    #[test]
    fn survives_reopening_rotates_atomically_and_deletes_only_selected_keys() {
        let root = tempfile::tempdir().unwrap();
        let first = CredentialStore::open(root.path()).unwrap();
        first.save("provider-one", b"first").unwrap();
        first.save("provider-two", b"second").unwrap();
        first.save("provider-one", b"rotated").unwrap();
        let reopened = CredentialStore::open(root.path()).unwrap();
        assert_eq!(reopened.load("provider-one").unwrap().unwrap(), b"rotated");
        reopened.delete("provider-one").unwrap();
        assert!(reopened.load("provider-one").unwrap().is_none());
        assert_eq!(reopened.load("provider-two").unwrap().unwrap(), b"second");
        reopened.clear().unwrap();
        assert!(reopened.load("provider-two").unwrap().is_none());
        assert!(first.directory.entries().unwrap().next().is_none());
    }
    #[test]
    fn reset_removes_orphan_records_and_stale_temps_but_not_unrelated_files() {
        let root = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(root.path()).unwrap();
        store.save("orphan", b"secret").unwrap();
        store
            .directory
            .write(".credential-stale.tmp", b"interrupted secret")
            .unwrap();
        store.directory.write("notes.txt", b"keep").unwrap();
        store.clear().unwrap();
        assert!(store.load("orphan").unwrap().is_none());
        assert!(!store.directory.exists(".credential-stale.tmp"));
        assert_eq!(store.directory.read("notes.txt").unwrap(), b"keep");
    }
    #[test]
    fn oversized_or_broken_records_fail_without_echoing_their_contents() {
        let root = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(root.path()).unwrap();
        assert!(
            store
                .save("secret-key-name", &vec![b'x'; MAX_SECRET_BYTES + 1])
                .is_err()
        );
        store.directory.create_dir(filename("bad")).unwrap();
        let error = store.save("bad", b"private-value").unwrap_err();
        assert!(!error.contains("private-value"));
        assert!(!error.contains("bad"));
        assert!(store.load("bad").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn file_and_directory_modes_are_private_and_symlinks_never_expose_targets() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(root.path()).unwrap();
        store.save("private", b"secret").unwrap();
        assert_eq!(
            std::fs::metadata(root.path().join("credentials"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(root.path().join("credentials").join(filename("private")))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"leave alone").unwrap();
        symlink(
            &outside,
            root.path().join("credentials").join(filename("linked")),
        )
        .unwrap();
        assert!(store.load("linked").is_err());
        assert!(store.save("linked", b"overwrite").is_err());
        store.delete("linked").unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"leave alone");
        let other = tempfile::tempdir().unwrap();
        symlink(other.path(), root.path().join("linked-root")).unwrap();
        assert!(CredentialStore::open(&root.path().join("linked-root")).is_err());
        let second = tempfile::tempdir().unwrap();
        symlink(other.path(), second.path().join("credentials")).unwrap();
        assert!(CredentialStore::open(second.path()).is_err());
        assert!(std::fs::read_dir(other.path()).unwrap().next().is_none());
    }
}
