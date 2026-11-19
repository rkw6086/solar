//! Private Orbit shell identity and startup behavior.
//!
//! Orbits reuse installed executables, but their shell startup files and CLI
//! credentials belong to their own workspace/configuration directories.

use crate::{managed_directory, managed_environment, managed_subdirectory};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellFamily {
    Posix,
    Zsh,
    Fish,
}

#[derive(Clone, Debug)]
struct Account {
    shell: PathBuf,
}

/// A user-invoked terminal session with the same private environment as an Orbit.
#[derive(Clone, Debug)]
pub struct OrbitShellSetup {
    pub shell: PathBuf,
    pub home: PathBuf,
    pub config_directory: PathBuf,
    pub command: String,
}

pub fn orbit_shell_setup(
    data_directory: &Path,
    workspace: &Path,
) -> Result<OrbitShellSetup, String> {
    setup_for_session(ShellSession::resolve(None, data_directory, workspace)?)
}

fn setup_for_session(session: ShellSession) -> Result<OrbitShellSetup, String> {
    let assignments = session
        .environment
        .iter()
        .map(|(key, value)| {
            let mut assignment = key.clone();
            assignment.push("=");
            assignment.push(value);
            shell_quote(&assignment)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let command = format!(
        "cd -- {} && exec /usr/bin/env -i {} {} -i -l",
        shell_quote(session.home.as_os_str())?,
        assignments.join(" "),
        shell_quote(session.executable.as_os_str())?,
    );
    Ok(OrbitShellSetup {
        shell: session.executable,
        home: session.home,
        config_directory: session.config_directory,
        command,
    })
}

fn shell_quote(value: &OsStr) -> Result<String, String> {
    let value = value
        .to_str()
        .ok_or("The terminal setup launcher requires UTF-8 paths and locale settings")?;
    if value.contains('\0') {
        return Err("The terminal setup launcher cannot contain null characters".into());
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

pub(crate) struct ShellSession {
    executable: PathBuf,
    family: ShellFamily,
    home: PathBuf,
    config_directory: PathBuf,
    private_bin: PathBuf,
    environment: Vec<(OsString, OsString)>,
}

impl ShellSession {
    pub(crate) fn resolve(
        explicit: Option<&str>,
        data_directory: &Path,
        workspace: &Path,
    ) -> Result<Self, String> {
        Self::from_snapshot(
            explicit.map(Path::new),
            current_account(),
            std::env::vars_os().collect(),
            data_directory,
            workspace,
        )
    }

    fn from_snapshot(
        explicit: Option<&Path>,
        account: Option<Account>,
        inherited: Vec<(OsString, OsString)>,
        data_directory: &Path,
        workspace: &Path,
    ) -> Result<Self, String> {
        let (executable, family) = if let Some(path) = explicit {
            let family = executable_family(path).ok_or(
                "Shell must be an absolute executable path to bash, zsh, sh, dash, ksh, or fish",
            )?;
            (path.to_path_buf(), family)
        } else {
            account
                .as_ref()
                .map(|account| account.shell.as_path())
                .into_iter()
                .chain(environment_value(&inherited, "SHELL").map(Path::new))
                .chain([Path::new("/bin/sh")])
                .find_map(|path| executable_family(path).map(|family| (path.to_path_buf(), family)))
                .ok_or("No supported shell is available")?
        };
        let home = managed_directory(workspace)?;
        let mut environment: BTreeMap<OsString, OsString> = managed_environment(data_directory)?
            .into_iter()
            .map(|(key, value)| (key.into(), value.into_os_string()))
            .collect();
        let config_directory = PathBuf::from(
            environment
                .get(OsStr::new("XDG_CONFIG_HOME"))
                .ok_or("Orbit configuration directory is unavailable")?
                .as_os_str(),
        );
        let glab_directory = managed_subdirectory(&config_directory, "glab-cli")?;
        let git_directory = managed_subdirectory(&config_directory, "git")?;
        remove_keychain_git_helpers(&git_directory.join("config"))?;
        let gcm_directory = managed_subdirectory(&config_directory, "gcm")?;
        let gcm_store = managed_subdirectory(&gcm_directory, "store")?;
        let (private_bin, ssh_config) = initialize_ssh(data_directory, &home)?;
        for (key, value) in &inherited {
            if safe_locale_key(key) || key == "TERM" {
                environment.insert(key.clone(), value.clone());
            }
        }
        // Only executable locations and display/locale settings cross from the
        // app environment. Tokens, credential helpers, SSH agents, startup-file
        // overrides, exported functions and arbitrary CLI config paths do not.
        environment.insert("PATH".into(), executable_path(&inherited, &private_bin)?);
        environment
            .entry("TERM".into())
            .or_insert_with(|| "dumb".into());
        environment.insert("HOME".into(), home.as_os_str().to_owned());
        environment.insert("USERPROFILE".into(), home.as_os_str().to_owned());
        environment.insert("SHELL".into(), executable.as_os_str().to_owned());
        environment.insert("ZDOTDIR".into(), home.as_os_str().to_owned());
        environment.insert("GLAB_CONFIG_DIR".into(), glab_directory.into_os_string());
        environment.insert("GLAB_USE_KEYRING".into(), "false".into());
        // GCM otherwise defaults to Keychain on macOS. Its file store stays
        // inside this Orbit, matching Solar's explicit file-storage preference.
        environment.insert("GCM_CREDENTIAL_STORE".into(), "plaintext".into());
        environment.insert(
            "GCM_PLAINTEXT_STORE_PATH".into(),
            gcm_store.into_os_string(),
        );
        environment.insert(
            "XDG_CONFIG_DIRS".into(),
            config_directory.as_os_str().to_owned(),
        );
        environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        environment.insert(
            "GIT_CONFIG_GLOBAL".into(),
            git_directory.join("config").into_os_string(),
        );
        environment.insert(
            "GIT_SSH_COMMAND".into(),
            format!(
                "/usr/bin/ssh {SSH_KEYCHAIN_OPTIONS} -F {}",
                shell_quote(ssh_config.as_os_str())?
            )
            .into(),
        );
        initialize_shell_profiles(&home, &config_directory, &private_bin)?;
        Ok(Self {
            executable,
            family,
            home,
            config_directory,
            private_bin,
            environment: environment.into_iter().collect(),
        })
    }

    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }

    pub(crate) fn arguments(&self, script: &str, cwd: &Path) -> Result<Vec<OsString>, String> {
        if !cwd.is_absolute() {
            return Err("Shell working directory must be an absolute path".into());
        }
        let mut arguments: Vec<OsString> =
            ["-i", "-l", "-c"].into_iter().map(OsString::from).collect();
        match self.family {
            ShellFamily::Posix => {
                arguments.push(OsString::from(
                    "__solar_command=\"$2\"\nPATH=\"$3:$PATH\"; export PATH\ncommand cd -- \"$1\" || exit\nset --\ncommand eval \"$__solar_command\"",
                ));
                arguments.push(OsString::from("solar"));
            }
            ShellFamily::Zsh => {
                arguments.push(OsString::from(
                    "__solar_command=\"$2\"\nPATH=\"$3:$PATH\"; export PATH\nbuiltin cd -- \"$1\" || exit\nbuiltin set --\nbuiltin eval \"$__solar_command\"",
                ));
                arguments.push(OsString::from("solar"));
            }
            ShellFamily::Fish => {
                arguments.push(OsString::from(
                    "set -l __solar_command \"$argv[2]\"\nset -gx PATH \"$argv[3]\" $PATH\nbuiltin cd -- \"$argv[1]\"; or exit $status\nset -e argv\nbuiltin eval -- \"$__solar_command\"",
                ));
            }
        }
        arguments.push(cwd.as_os_str().to_owned());
        arguments.push(OsString::from(script));
        arguments.push(self.private_bin.as_os_str().to_owned());
        Ok(arguments)
    }

    pub(crate) fn apply_environment(&self, command: &mut tokio::process::Command) {
        command.env_clear().envs(self.environment.iter().cloned());
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        shell: &str,
        data_directory: &Path,
        workspace: &Path,
        inherited: Vec<(OsString, OsString)>,
    ) -> Result<Self, String> {
        Self::from_snapshot(
            Some(Path::new(shell)),
            None,
            inherited,
            data_directory,
            workspace,
        )
    }
}

fn safe_locale_key(key: &OsStr) -> bool {
    matches!(
        key.to_str(),
        Some(
            "LANG"
                | "LC_ALL"
                | "LC_CTYPE"
                | "LC_COLLATE"
                | "LC_MESSAGES"
                | "LC_MONETARY"
                | "LC_NUMERIC"
                | "LC_TIME"
                | "LC_ADDRESS"
                | "LC_IDENTIFICATION"
                | "LC_MEASUREMENT"
                | "LC_NAME"
                | "LC_PAPER"
                | "LC_TELEPHONE"
        )
    )
}

fn executable_path(
    inherited: &[(OsString, OsString)],
    private_bin: &Path,
) -> Result<OsString, String> {
    let mut seen = BTreeSet::from([private_bin.to_path_buf()]);
    let mut paths = vec![private_bin.to_path_buf()];
    if let Some(path) = environment_value(inherited, "PATH") {
        for path in std::env::split_paths(path).filter(|path| path.is_absolute()) {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    // Finder/launchd commonly omit Homebrew and /usr/local from PATH. Appending
    // standard locations makes installed CLIs discoverable without importing
    // the user's shell profiles or credential environment.
    for path in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        let path = PathBuf::from(path);
        if seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    std::env::join_paths(paths).map_err(|_| "Cannot construct the Orbit executable path".into())
}

fn initialize_shell_profiles(
    home: &Path,
    config_directory: &Path,
    private_bin: &Path,
) -> Result<(), String> {
    let path_prefix = format!(
        "PATH={}:\"$PATH\"; export PATH\n",
        shell_quote(private_bin.as_os_str())?
    );
    initialize_bash_profile(home, &path_prefix)?;
    write_private_file_if_missing(&home.join(".zshrc"), path_prefix.as_bytes(), 0o600)?;
    write_private_file_if_missing(&home.join(".profile"), path_prefix.as_bytes(), 0o600)?;
    let fish = managed_subdirectory(config_directory, "fish")?;
    let fish_prefix = format!(
        "set -gx PATH {} $PATH\n",
        shell_quote(private_bin.as_os_str())?
    );
    write_private_file_if_missing(&fish.join("config.fish"), fish_prefix.as_bytes(), 0o600)?;
    Ok(())
}

fn initialize_bash_profile(home: &Path, path_prefix: &str) -> Result<(), String> {
    for name in [".bash_profile", ".bash_login", ".profile"] {
        match std::fs::symlink_metadata(home.join(name)) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot inspect Orbit shell startup files: {error}")),
        }
    }
    let contents = format!(
        "# Solar's private login profile.\nif [ -f \"$HOME/.bashrc\" ]; then\n    . \"$HOME/.bashrc\"\nfi\n{path_prefix}"
    );
    write_private_file_if_missing(&home.join(".bash_profile"), contents.as_bytes(), 0o600)
}

// Command-line options take precedence over imported SSH configuration. The
// IgnoreUnknown option makes this portable to non-Apple OpenSSH builds.
const SSH_KEYCHAIN_OPTIONS: &str = "-o IgnoreUnknown=UseKeychain -o UseKeychain=no";

fn initialize_ssh(data_directory: &Path, home: &Path) -> Result<(PathBuf, PathBuf), String> {
    let ssh = managed_directory(&home.join(".ssh"))?;
    let config = ssh.join("config");
    let contents = format!(
        "# Solar's private SSH defaults.\nHost *\n    IdentityAgent none\n    IdentitiesOnly yes\n    IdentityFile {}\n    IdentityFile {}\n    UserKnownHostsFile {}\n    IgnoreUnknown UseKeychain\n    UseKeychain no\n",
        ssh_config_path(&ssh.join("id_ed25519"))?,
        ssh_config_path(&ssh.join("id_rsa"))?,
        ssh_config_path(&ssh.join("known_hosts"))?,
    );
    write_private_file_if_missing(&config, contents.as_bytes(), 0o600)?;
    let shell = managed_subdirectory(data_directory, "shell")?;
    let private_bin = managed_subdirectory(&shell, "bin")?;
    for name in ["ssh", "scp", "sftp"] {
        let executable = Path::new("/usr/bin").join(name);
        if executable.is_file() {
            let legacy = format!(
                "#!/bin/sh\nexec {} -F {} \"$@\"\n",
                shell_quote(executable.as_os_str())?,
                shell_quote(config.as_os_str())?,
            );
            let wrapper = format!(
                "#!/bin/sh\nexec {} {SSH_KEYCHAIN_OPTIONS} -F {} \"$@\"\n",
                shell_quote(executable.as_os_str())?,
                shell_quote(config.as_os_str())?,
            );
            update_generated_wrapper(
                &private_bin.join(name),
                wrapper.as_bytes(),
                legacy.as_bytes(),
            )?;
        }
    }
    Ok((private_bin, config))
}

/// Upgrade only the exact wrapper emitted by Solar's previous version. A
/// custom wrapper or symlink is owned by the user and is left alone.
fn update_generated_wrapper(path: &Path, contents: &[u8], previous: &[u8]) -> Result<(), String> {
    use std::io::Read;
    write_private_file_if_missing(path, contents, 0o700)?;
    let read_previous = || -> Result<bool, String> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| "Cannot inspect Orbit SSH wrapper".to_owned())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != previous.len() as u64
        {
            return Ok(false);
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options
            .open(path)
            .map_err(|_| "Cannot read Orbit SSH wrapper".to_owned())?;
        if !file
            .metadata()
            .map_err(|_| "Cannot inspect Orbit SSH wrapper")?
            .is_file()
        {
            return Ok(false);
        }
        let mut actual = Vec::new();
        file.take(previous.len() as u64 + 1)
            .read_to_end(&mut actual)
            .map_err(|_| "Cannot read Orbit SSH wrapper".to_owned())?;
        Ok(actual == previous)
    };
    if !read_previous()? {
        return Ok(());
    }
    let temporary = path.with_file_name(format!(".solar-ssh-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o700);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| "Cannot update Orbit SSH wrapper".to_owned())?;
        file.write_all(contents)
            .map_err(|_| "Cannot update Orbit SSH wrapper".to_owned())?;
        file.sync_all()
            .map_err(|_| "Cannot save Orbit SSH wrapper".to_owned())?;
        if read_previous()? {
            std::fs::rename(&temporary, path)
                .map_err(|_| "Cannot finish Orbit SSH wrapper update".to_owned())?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

// Only the Orbit's own configuration is changed. Parse a private snapshot so
// Git never follows user includes or writes through the original pathname.
fn remove_keychain_git_helpers(path: &Path) -> Result<(), String> {
    const MAX_CONFIG: usize = 1_048_576;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let Some(original) = read_private_git_config(path, MAX_CONFIG)? else {
        return Ok(());
    };
    // Most shells never need a Git process. The substring is only a cheap
    // prefilter; Git itself decides which lines are real helper assignments.
    if !original
        .windows(11)
        .any(|value| value.eq_ignore_ascii_case(b"osxkeychain"))
    {
        return Ok(());
    }
    let Some(git) = [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/usr/bin/git",
        "/bin/git",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file()) else {
        return Err("Git is needed to update this Orbit's imported Git configuration".into());
    };
    let parent = path
        .parent()
        .ok_or("Invalid private Git configuration path")?;
    let temporary = parent.join(format!(".solar-git-{}.config", uuid::Uuid::new_v4()));
    struct Snapshot(PathBuf);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("config.lock"));
        }
    }
    let _snapshot = Snapshot(temporary.clone());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "Cannot inspect private Git configuration")?;
    file.write_all(&original)
        .map_err(|_| "Cannot inspect private Git configuration")?;
    file.sync_all()
        .map_err(|_| "Cannot inspect private Git configuration")?;
    drop(file);
    let (status, output) = git_config_command(
        &git,
        &temporary,
        &["--null", "--get-regexp", r"^credential(\..*)?\.helper$"],
        deadline,
    )?;
    if status.code() == Some(1) {
        return Ok(());
    }
    if !status.success() {
        return Err("The Orbit's Git configuration could not be read. Check its syntax.".into());
    }
    let text = std::str::from_utf8(&output).map_err(|_| "Git configuration must use UTF-8 text")?;
    let mut removed = BTreeSet::new();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let Some((key, value)) = entry.split_once('\n') else {
            return Err("Cannot inspect private Git credential helpers".into());
        };
        if value.to_ascii_lowercase().contains("osxkeychain") {
            removed.insert((key.to_owned(), value.to_owned()));
        }
    }
    if removed.is_empty() {
        return Ok(());
    }
    if removed.len() > 128 {
        return Err("Orbit Git configuration has too many Keychain helper entries".into());
    }
    for (key, value) in removed {
        let (status, _) = git_config_command(
            &git,
            &temporary,
            &["--fixed-value", "--unset-all", &key, &value],
            deadline,
        )?;
        if !status.success() {
            return Err(
                "Cannot remove a Keychain helper from the Orbit's Git configuration".into(),
            );
        }
    }
    if read_private_git_config(&temporary, MAX_CONFIG)?.is_none() {
        return Err("Private Git configuration disappeared".into());
    }
    if read_private_git_config(path, MAX_CONFIG)?.as_deref() != Some(original.as_slice()) {
        return Err("The Orbit's Git configuration changed during its update. Try again.".into());
    }
    // Git preserves unrelated lines/comments. Ensure its rewritten copy is
    // private before the atomic replacement, regardless of the process umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "Cannot protect private Git configuration")?;
    }
    // Flush the final Git-written contents before moving them into place.
    std::fs::OpenOptions::new()
        .read(true)
        .open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Cannot save private Git configuration")?;
    std::fs::rename(&temporary, path).map_err(|_| "Cannot update private Git configuration")?;
    Ok(())
}

fn read_private_git_config(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot inspect private Git configuration".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Orbit Git configuration must be a regular private file".into());
    }
    if metadata.len() > maximum as u64 {
        return Err("Orbit Git configuration exceeds the 1 MiB limit".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "Cannot read private Git configuration")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect private Git configuration")?
        .is_file()
    {
        return Err("Orbit Git configuration must be a regular private file".into());
    }
    let mut output = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|_| "Cannot read private Git configuration")?;
    if output.len() > maximum {
        return Err("Orbit Git configuration exceeds the 1 MiB limit".into());
    }
    Ok(Some(output))
}

fn git_config_command(
    git: &Path,
    path: &Path,
    arguments: &[&str],
    deadline: std::time::Instant,
) -> Result<(std::process::ExitStatus, Vec<u8>), String> {
    use std::{
        io::Read,
        process::Stdio,
        time::{Duration, Instant},
    };
    const MAX_OUTPUT: usize = 2_097_152;
    if Instant::now() >= deadline {
        return Err("Git configuration inspection timed out".into());
    }
    let mut child = std::process::Command::new(git)
        .arg("config")
        .arg("--file")
        .arg(path)
        .arg("--no-includes")
        .args(arguments)
        .current_dir(
            path.parent()
                .ok_or("Invalid private Git configuration path")?,
        )
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot run Git to inspect private configuration")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Cannot read Git configuration result")?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut reader = stdout;
        let mut buffer = [0; 8192];
        let mut exceeded = false;
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|_| "Cannot read Git configuration result")?;
            if count == 0 {
                break;
            }
            let keep = count.min(MAX_OUTPUT.saturating_sub(output.len()));
            output.extend_from_slice(&buffer[..keep]);
            exceeded |= keep < count;
        }
        if exceeded {
            Err("Private Git configuration result exceeded its limit")
        } else {
            Ok(output)
        }
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("Git configuration inspection timed out or was interrupted");
            }
        }
    };
    let output = reader
        .join()
        .map_err(|_| "Git configuration inspection was interrupted")??;
    Ok((status?, output))
}

fn ssh_config_path(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("SSH setup requires UTF-8 paths")?;
    if path.chars().any(char::is_control) || path.contains("${") {
        return Err(
            "SSH setup cannot use control characters or environment expansions in paths".into(),
        );
    }
    Ok(format!(
        "\"{}\"",
        path.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}

fn write_private_file_if_missing(path: &Path, contents: &[u8], mode: u32) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(format!("Cannot create Orbit shell startup file: {error}")),
    };
    if let Err(error) = file.write_all(contents) {
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "Cannot initialize Orbit shell startup file: {error}"
        ));
    }
    Ok(())
}

fn environment_value<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    environment
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_os_str())
}

fn executable_family(path: &Path) -> Option<ShellFamily> {
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.metadata().ok()?.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    match path.file_name()?.to_str()? {
        "bash" | "sh" | "dash" | "ksh" | "ksh93" => Some(ShellFamily::Posix),
        "zsh" => Some(ShellFamily::Zsh),
        "fish" => Some(ShellFamily::Fish),
        _ => None,
    }
}

#[cfg(unix)]
fn current_account() -> Option<Account> {
    use std::{ffi::CStr, os::unix::ffi::OsStrExt};

    const MAX_BUFFER: usize = 1_048_576;
    let hint = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut size = if hint > 0 {
        (hint as usize).clamp(1024, MAX_BUFFER)
    } else {
        16_384
    };
    loop {
        let mut buffer = vec![0_u8; size];
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found = std::ptr::null_mut();
        let result = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if result == libc::ERANGE && size < MAX_BUFFER {
            size = (size * 2).min(MAX_BUFFER);
            continue;
        }
        if result != 0 || found.is_null() {
            return None;
        }
        let entry = unsafe { entry.assume_init() };
        if entry.pw_shell.is_null() {
            return None;
        }
        let shell = unsafe { CStr::from_ptr(entry.pw_shell) };
        return Some(Account {
            shell: PathBuf::from(OsStr::from_bytes(shell.to_bytes())),
        });
    }
}

#[cfg(not(unix))]
fn current_account() -> Option<Account> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_root() -> tempfile::TempDir {
        tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
    }

    fn fake_shell(root: &Path, name: &str) -> PathBuf {
        let shell = root.join(name);
        std::fs::write(&shell, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        shell
    }

    #[test]
    fn account_shell_precedes_environment_with_private_home() {
        let root = private_root();
        let account_shell = fake_shell(root.path(), "zsh");
        let env_shell = fake_shell(root.path(), "bash");
        let workspace = root.path().join("workspace");
        let session = ShellSession::from_snapshot(
            None,
            Some(Account {
                shell: account_shell.clone(),
            }),
            vec![
                ("SHELL".into(), env_shell.into_os_string()),
                ("HOME".into(), "/personal/home".into()),
            ],
            root.path(),
            &workspace,
        )
        .unwrap();
        assert_eq!(session.executable(), account_shell);
        assert_eq!(session.home, workspace);
    }

    #[test]
    fn explicit_invalid_shell_does_not_fall_back() {
        let root = private_root();
        let shell = fake_shell(root.path(), "bash");
        let unknown = fake_shell(root.path(), "not-a-shell");
        let inherited = vec![("SHELL".into(), shell.into_os_string())];
        for explicit in [Path::new("bash"), unknown.as_path(), root.path()] {
            assert!(
                ShellSession::from_snapshot(
                    Some(explicit),
                    None,
                    inherited.clone(),
                    root.path(),
                    &root.path().join("workspace")
                )
                .is_err()
            );
        }
    }

    #[test]
    fn invalid_account_shell_uses_inherited_shell_then_safe_fallback() {
        let root = private_root();
        let fish = fake_shell(root.path(), "fish");
        let session = ShellSession::from_snapshot(
            None,
            Some(Account {
                shell: "/missing/shell".into(),
            }),
            vec![("SHELL".into(), fish.clone().into_os_string())],
            root.path(),
            &root.path().join("workspace"),
        )
        .unwrap();
        assert_eq!(session.executable(), fish);
        assert_eq!(session.family, ShellFamily::Fish);
        let fallback = ShellSession::from_snapshot(
            None,
            None,
            vec![],
            root.path(),
            &root.path().join("workspace"),
        )
        .unwrap();
        assert_eq!(fallback.executable(), Path::new("/bin/sh"));
    }

    #[test]
    fn personal_credentials_and_startup_overrides_never_enter_private_environment() {
        let root = private_root();
        let shell = fake_shell(root.path(), "zsh");
        let workspace = root.path().join("workspace");
        let inherited: Vec<(OsString, OsString)> = [
            ("HOME", "/personal/home"),
            ("GLAB_TOKEN", "secret"),
            ("GITLAB_TOKEN", "secret"),
            ("SSH_AUTH_SOCK", "/personal/agent"),
            ("GLAB_CONFIG_DIR", "/personal/glab"),
            ("GLAB_USE_KEYRING", "true"),
            ("GCM_CREDENTIAL_STORE", "keychain"),
            ("GCM_PLAINTEXT_STORE_PATH", "/personal/gcm"),
            ("GIT_CONFIG_GLOBAL", "/personal/gitconfig"),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "credential.helper"),
            ("GIT_CONFIG_VALUE_0", "osxkeychain"),
            ("BASH_ENV", "/personal/bashenv"),
            ("ENV", "/personal/shenv"),
            ("ZDOTDIR", "/personal/zsh"),
            ("XDG_CONFIG_HOME", "/personal/config"),
            ("XDG_CONFIG_DIRS", "/personal/shared-config"),
            ("AWS_PROFILE", "personal"),
            ("RYSERVE_LOGGING_KEY", "secret"),
            ("BASH_FUNC_leak%%", "() { echo secret; }"),
            ("LC_FAKE_SECRET", "secret"),
            ("PATH", "/custom/bin:/usr/bin"),
            ("TERM", "xterm-256color"),
            ("LANG", "en_US.UTF-8"),
            ("LC_TIME", "en_US.UTF-8"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let session =
            ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, inherited)
                .unwrap();
        let env: BTreeMap<_, _> = session.environment.iter().cloned().collect();
        for name in [
            "GLAB_TOKEN",
            "GITLAB_TOKEN",
            "SSH_AUTH_SOCK",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_VALUE_0",
            "BASH_ENV",
            "ENV",
            "AWS_PROFILE",
            "RYSERVE_LOGGING_KEY",
            "BASH_FUNC_leak%%",
            "LC_FAKE_SECRET",
        ] {
            assert!(!env.contains_key(OsStr::new(name)), "inherited {name}");
        }
        for name in ["HOME", "ZDOTDIR", "USERPROFILE"] {
            assert_eq!(env[OsStr::new(name)], workspace.as_os_str());
        }
        assert_eq!(
            env[OsStr::new("GLAB_CONFIG_DIR")],
            root.path().join("config/glab-cli").as_os_str()
        );
        assert_eq!(env[OsStr::new("GLAB_USE_KEYRING")], "false");
        assert_eq!(env[OsStr::new("GCM_CREDENTIAL_STORE")], "plaintext");
        assert_eq!(
            env[OsStr::new("GCM_PLAINTEXT_STORE_PATH")],
            root.path().join("config/gcm/store").as_os_str()
        );
        assert_eq!(
            env[OsStr::new("XDG_CONFIG_DIRS")],
            root.path().join("config").as_os_str()
        );
        assert_eq!(
            env[OsStr::new("GIT_CONFIG_GLOBAL")],
            root.path().join("config/git/config").as_os_str()
        );
        assert_eq!(env[OsStr::new("GIT_CONFIG_NOSYSTEM")], "1");
        assert_eq!(env[OsStr::new("TERM")], "xterm-256color");
        assert_eq!(env[OsStr::new("LC_TIME")], "en_US.UTF-8");
        for name in [
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "TMPDIR",
        ] {
            assert!(Path::new(&env[OsStr::new(name)]).starts_with(root.path()));
        }
        let paths: Vec<_> = std::env::split_paths(&env[OsStr::new("PATH")]).collect();
        assert_eq!(paths[0], root.path().join("shell/bin"));
        assert_eq!(paths[1], Path::new("/custom/bin"));
        assert!(paths.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(paths.contains(&PathBuf::from("/usr/local/bin")));
    }

    #[test]
    fn bash_profile_initializes_once_and_respects_any_existing_login_profile() {
        let root = private_root();
        let shell = fake_shell(root.path(), "bash");
        let workspace = root.path().join("workspace");
        ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![]).unwrap();
        let profile = workspace.join(".bash_profile");
        assert!(
            std::fs::read_to_string(&profile)
                .unwrap()
                .contains("\"$HOME/.bashrc\"")
        );
        std::fs::write(&profile, "# user's private configuration\n").unwrap();
        ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&profile).unwrap(),
            "# user's private configuration\n"
        );
        for existing in [".bash_login", ".profile"] {
            let separate = root.path().join(existing.trim_start_matches('.'));
            std::fs::create_dir(&separate).unwrap();
            std::fs::write(separate.join(existing), "# existing login\n").unwrap();
            ShellSession::for_test(shell.to_str().unwrap(), root.path(), &separate, vec![])
                .unwrap();
            assert!(!separate.join(".bash_profile").exists());
            assert_eq!(
                std::fs::read_to_string(separate.join(existing)).unwrap(),
                "# existing login\n"
            );
        }
    }

    #[test]
    fn arguments_preserve_literals_and_fish_positional_rules() {
        let root = private_root();
        let workspace = root.path().join("workspace");
        let shell = fake_shell(root.path(), "zsh");
        let session =
            ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![])
                .unwrap();
        let cwd = Path::new("/tmp/a 'quoted' $(directory)");
        let script = "printf '%s\\n' \"a 'quote'\" \"$HOME\"\n";
        let args = session.arguments(script, cwd).unwrap();
        assert_eq!(
            &args[..3],
            &[
                OsString::from("-i"),
                OsString::from("-l"),
                OsString::from("-c")
            ]
        );
        assert_eq!(args[4], "solar");
        assert_eq!(args[5], cwd.as_os_str());
        assert_eq!(args[6], script);
        assert_eq!(args[7], root.path().join("shell/bin").as_os_str());
        assert!(args[3].to_string_lossy().contains("builtin cd"));
        assert!(session.arguments("pwd", Path::new("relative")).is_err());
        let fish = fake_shell(root.path(), "fish");
        let fish = ShellSession::for_test(fish.to_str().unwrap(), root.path(), &workspace, vec![])
            .unwrap();
        let args = fish.arguments(script, cwd).unwrap();
        assert_eq!(args.len(), 7);
        assert!(args[3].to_string_lossy().contains("$argv[1]"));
        assert_eq!(args[4], cwd.as_os_str());
        assert_eq!(args[5], script);
    }

    #[test]
    fn setup_launcher_uses_only_private_environment_and_quotes_paths() {
        let root = private_root();
        let workspace = root.path().join("a 'quoted' $(workspace)");
        let shell = fake_shell(root.path(), "bash");
        let session = ShellSession::for_test(
            shell.to_str().unwrap(),
            root.path(),
            &workspace,
            vec![("GLAB_TOKEN".into(), "do-not-copy".into())],
        )
        .unwrap();
        let setup = setup_for_session(session).unwrap();
        assert_eq!(setup.home, workspace);
        assert_eq!(setup.config_directory, root.path().join("config"));
        assert!(setup.command.contains("exec /usr/bin/env -i "));
        assert!(setup.command.contains("'\\''"));
        assert!(setup.command.ends_with(" -i -l"));
        assert!(!setup.command.contains("do-not-copy"));
        assert!(!setup.command.contains("GLAB_TOKEN="));
        assert_eq!(shell_quote(OsStr::new("a'b")).unwrap(), "'a'\\''b'");
    }

    #[cfg(unix)]
    #[test]
    fn internal_launcher_arguments_are_not_visible_to_the_command() {
        let mut shells = BTreeSet::from([PathBuf::from("/bin/sh")]);
        for name in ["bash", "zsh", "fish"] {
            if let Some(path) = std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|path| path.join(name))
                    .find(|path| executable_family(path).is_some())
            }) {
                shells.insert(path);
            }
        }
        for shell in shells {
            let root = private_root();
            let workspace = root.path().join("workspace");
            let session =
                ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![])
                    .unwrap();
            let script = if session.family == ShellFamily::Fish {
                "printf '__solar_argc=%s\\n' (count $argv)"
            } else {
                "printf '__solar_argc=%s\\n' \"$#\""
            };
            let mut command = tokio::process::Command::new(session.executable());
            session.apply_environment(&mut command);
            let output = command
                .as_std_mut()
                .args(session.arguments(script, &workspace).unwrap())
                .current_dir(&workspace)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}: {}",
                shell.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line == "__solar_argc=0"),
                "{}: {}",
                shell.display(),
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    #[test]
    fn private_git_keychain_cleanup_preserves_other_helpers_and_original_source() {
        let root = private_root();
        let source = root.path().join("original.gitconfig");
        let config = root.path().join("config");
        let included = root.path().join("included.gitconfig");
        let content = format!(
            "# keep this comment\n[user]\n\tname = Orbit\n[credential]\n\thelper = osxkeychain\n\thelper = !glab auth git-credential\n[credential \"https://github.com\"]\n\thelper = !gh auth git-credential\n\thelper = /usr/bin/git-credential-osxkeychain\n[include]\n\tpath = {}\n",
            included.display()
        );
        std::fs::write(&source, &content).unwrap();
        std::fs::write(&config, &content).unwrap();
        std::fs::write(&included, "[credential]\nhelper = osxkeychain\n").unwrap();
        remove_keychain_git_helpers(&config).unwrap();
        let updated = std::fs::read_to_string(&config).unwrap();
        assert!(!updated.contains("helper = osxkeychain"));
        assert!(!updated.contains("git-credential-osxkeychain"));
        assert!(updated.contains("!glab auth git-credential"));
        assert!(updated.contains("!gh auth git-credential"));
        assert!(updated.contains("# keep this comment"));
        assert_eq!(std::fs::read_to_string(&source).unwrap(), content);
        assert!(
            std::fs::read_to_string(&included)
                .unwrap()
                .contains("osxkeychain")
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // A comment about Keychain must not trigger any rewriting by itself.
        std::fs::write(
            &config,
            "# osxkeychain is disabled\n[credential]\nhelper = !glab auth git-credential\n",
        )
        .unwrap();
        let unchanged = std::fs::read(&config).unwrap();
        remove_keychain_git_helpers(&config).unwrap();
        assert_eq!(std::fs::read(&config).unwrap(), unchanged);
    }

    #[cfg(unix)]
    #[test]
    fn private_git_keychain_cleanup_rejects_symlinks_and_does_not_leak_errors() {
        let root = private_root();
        let config = root.path().join("config");
        let outside = root.path().join("outside");
        std::fs::write(&outside, "[credential]\nhelper = osxkeychain\n").unwrap();
        std::os::unix::fs::symlink(&outside, &config).unwrap();
        assert!(remove_keychain_git_helpers(&config).is_err());
        assert!(
            std::fs::read_to_string(&outside)
                .unwrap()
                .contains("osxkeychain")
        );
        std::fs::remove_file(&config).unwrap();
        std::fs::write(
            &config,
            "private-secret-osxkeychain invalid git configuration",
        )
        .unwrap();
        let error = remove_keychain_git_helpers(&config).unwrap_err();
        assert!(!error.contains("private-secret"));
    }

    #[test]
    fn only_unchanged_solar_ssh_wrappers_are_upgraded() {
        let root = private_root();
        let wrapper = root.path().join("ssh");
        let previous = b"#!/bin/sh\nexec /usr/bin/ssh -F private \"$@\"\n";
        let current = b"#!/bin/sh\nexec /usr/bin/ssh -o UseKeychain=no -F private \"$@\"\n";
        std::fs::write(&wrapper, previous).unwrap();
        update_generated_wrapper(&wrapper, current, previous).unwrap();
        assert_eq!(std::fs::read(&wrapper).unwrap(), current);
        std::fs::write(&wrapper, b"#!/bin/sh\n# custom wrapper\n").unwrap();
        update_generated_wrapper(&wrapper, current, previous).unwrap();
        assert_eq!(
            std::fs::read(&wrapper).unwrap(),
            b"#!/bin/sh\n# custom wrapper\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let target = root.path().join("user-target");
            std::fs::write(&target, previous).unwrap();
            std::fs::remove_file(&wrapper).unwrap();
            symlink(&target, &wrapper).unwrap();
            update_generated_wrapper(&wrapper, current, previous).unwrap();
            assert!(
                std::fs::symlink_metadata(&wrapper)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read(&target).unwrap(), previous);
            let fresh = root.path().join("fresh");
            update_generated_wrapper(&fresh, current, previous).unwrap();
            assert_eq!(
                std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn imported_ssh_settings_cannot_reenable_keychain() {
        if !Path::new("/usr/bin/ssh").is_file() {
            return;
        }
        let root = private_root();
        let shell = fake_shell(root.path(), "bash");
        let workspace = root.path().join("workspace");
        ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![]).unwrap();
        let config = workspace.join(".ssh/config");
        std::fs::write(
            &config,
            "IgnoreUnknown UseKeychain\nHost *\n    UseKeychain yes\n",
        )
        .unwrap();
        let session =
            ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![])
                .unwrap();
        assert!(
            session
                .environment
                .iter()
                .any(|(key, value)| key == "GIT_SSH_COMMAND"
                    && value.to_string_lossy().contains(SSH_KEYCHAIN_OPTIONS))
        );
        let wrapper = root.path().join("shell/bin/ssh");
        assert!(
            std::fs::read_to_string(&wrapper)
                .unwrap()
                .contains(SSH_KEYCHAIN_OPTIONS)
        );
        let output = std::process::Command::new(&wrapper)
            .args(["-o", "UseKeychain=yes", "-G", "example.invalid"])
            .env_clear()
            .envs(session.environment.iter().cloned())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        #[cfg(target_os = "macos")]
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line == "usekeychain no")
        );
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("UseKeychain yes")
        );
    }

    #[cfg(unix)]
    #[test]
    fn ssh_defaults_resolve_only_private_identity_and_known_hosts_paths() {
        use std::os::unix::fs::PermissionsExt;

        if !Path::new("/usr/bin/ssh").is_file() {
            return;
        }
        let root = private_root();
        let shell = fake_shell(root.path(), "bash");
        let workspace = root.path().join("Orbit's SSH files");
        let session =
            ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![])
                .unwrap();
        let wrapper = root.path().join("shell/bin/ssh");
        let output = std::process::Command::new(&wrapper)
            .args(["-G", "example.invalid"])
            .env_clear()
            .envs(session.environment.iter().cloned())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config = String::from_utf8(output.stdout).unwrap();
        assert!(config.lines().any(|line| line == "identityagent none"));
        assert!(config.lines().any(|line| line == "identitiesonly yes"));
        let identities: Vec<_> = config
            .lines()
            .filter_map(|line| line.strip_prefix("identityfile "))
            .collect();
        assert_eq!(
            identities,
            vec![
                workspace.join(".ssh/id_ed25519").to_str().unwrap(),
                workspace.join(".ssh/id_rsa").to_str().unwrap(),
            ]
        );
        assert!(config.lines().any(|line| line
            == format!(
                "userknownhostsfile {}",
                workspace.join(".ssh/known_hosts").display()
            )));
        assert_eq!(
            std::fs::metadata(&wrapper).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(workspace.join(".ssh/config"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(workspace.join(".ssh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let path = workspace.join(".ssh/config");
        std::fs::write(&path, "# my private SSH configuration\n").unwrap();
        ShellSession::for_test(shell.to_str().unwrap(), root.path(), &workspace, vec![]).unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "# my private SSH configuration\n"
        );
        assert!(ssh_config_path(Path::new("/tmp/new\nline")).is_err());
        assert!(ssh_config_path(Path::new("/tmp/${HOME}")).is_err());
        assert_eq!(
            ssh_config_path(Path::new("/tmp/a%h\"b")).unwrap(),
            "\"/tmp/a%%h\\\"b\""
        );
    }
}
