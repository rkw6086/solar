use crate::{engine::Engine, model::id, profile_config::secure_directory};
use serde::Serialize;
use std::{fs::OpenOptions, io::Write, path::PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrbitCliSetup {
    shell: String,
    home: String,
    config_directory: String,
    command: String,
}

fn prepare(engine: &Engine, orbit_id: &str) -> Result<(OrbitCliSetup, PathBuf), String> {
    engine.ensure_running()?;
    engine.orbit(orbit_id)?;
    let root = engine.store.orbit_dir(orbit_id)?;
    let workspace = engine.workspace(orbit_id)?;
    secure_directory(&workspace)?;
    engine.sync_workspace(orbit_id)?;
    let setup = solar_tools::orbit_shell_setup(&root, &workspace)?;
    let directory = root.join("shell");
    secure_directory(&directory)?;
    Ok((
        OrbitCliSetup {
            shell: setup.shell.to_string_lossy().into_owned(),
            home: setup.home.to_string_lossy().into_owned(),
            config_directory: setup.config_directory.to_string_lossy().into_owned(),
            command: setup.command,
        },
        directory.join("CLI Setup.command"),
    ))
}

pub(crate) fn describe(engine: &Engine, orbit_id: &str) -> Result<OrbitCliSetup, String> {
    prepare(engine, orbit_id).map(|(setup, _)| setup)
}

fn write_launcher(path: &std::path::Path, command: &str) -> Result<(), String> {
    let temporary = path.with_file_name(format!(".cli-{}.tmp", id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o700);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        writeln!(file, "#!/bin/sh\n{command}").map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

pub(crate) async fn open(engine: &Engine, orbit_id: &str) -> Result<(), String> {
    let (setup, launcher) = prepare(engine, orbit_id)?;
    write_launcher(&launcher, &setup.command)?;
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = tokio::process::Command::new("/usr/bin/open");
        command.args(["-a", "Terminal"]).arg(&launcher);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let candidates = [
            ("/usr/bin/x-terminal-emulator", "-e"),
            ("/usr/bin/gnome-terminal", "--"),
            ("/usr/bin/konsole", "-e"),
            ("/usr/bin/xterm", "-e"),
        ];
        let (executable, separator) = candidates.into_iter()
            .find(|(executable, _)| std::path::Path::new(executable).is_file())
            .ok_or("No desktop terminal is available. Copy the setup command into your terminal instead.")?;
        let mut command = tokio::process::Command::new(executable);
        command.arg(separator).arg("/bin/sh").arg(&launcher);
        command
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // Terminal owns the interactive session after launch. This user-operated
        // setup terminal is separate from the Orbit's scoped, timed tool calls.
        let mut child = command
            .spawn()
            .map_err(|_| "Could not open a terminal. Copy the setup command instead.".to_owned())?;
        #[cfg(target_os = "macos")]
        {
            let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
                .await
                .map_err(|_| {
                    "Terminal is taking too long to open. You can copy the setup command instead."
                        .to_owned()
                })?
                .map_err(|_| {
                    "Could not open Terminal. Copy the setup command instead.".to_owned()
                })?;
            if status.success() {
                Ok(())
            } else {
                Err("Could not open Terminal. Copy the setup command instead.".into())
            }
        }
        #[cfg(target_os = "linux")]
        {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(())
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Err("Automatic terminal launch is unavailable. Copy the setup command into your terminal instead.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_uses_the_orbits_own_paths_and_never_adds_an_orbit() {
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::open(directory.path().into()).unwrap();
        assert!(describe(&engine, "missing").is_err());
        assert!(engine.bootstrap().unwrap().orbits.is_empty());
        crate::test_support::add_assistant(&engine);
        let setup = describe(&engine, "assistant").unwrap();
        let root = engine.store.orbit_dir("assistant").unwrap();
        assert_eq!(PathBuf::from(setup.home), root.join("workspace"));
        let config = PathBuf::from(setup.config_directory);
        assert_eq!(config, root.join("workspace/.config"));
        assert_eq!(
            config.canonicalize().unwrap(),
            root.join("config").canonicalize().unwrap()
        );
        assert!(setup.command.contains("env -i"));
        assert!(!setup.command.contains("glab auth login"));
    }

    #[test]
    fn launcher_replacement_does_not_follow_a_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("setup.command");
        let outside = directory.path().join("keep.txt");
        std::fs::write(&outside, "keep").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        write_launcher(&path, "exec /bin/sh").unwrap();
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "#!/bin/sh\nexec /bin/sh\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
}
