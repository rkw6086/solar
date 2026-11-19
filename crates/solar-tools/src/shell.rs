use crate::{ToolContext, ToolDefinition, ToolOutput, ToolProgress, definition, required_str};
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![definition(
        "shell_run",
        "Run a command in the user's default shell with this Sprite's own interactive/login startup files and private CLI configuration. User-level credentials and SSH agents are not inherited. When authentication or profile setup is missing, request the specific needed files with pull_from_user_home (requires explicit user approval), or ask the user to use this Sprite's CLI sign-ins settings. Optional workingDirectory must be the Sprite workspace or an assigned project folder. Unassigned registered project folders are excluded from the shell, its startup files, and its descendants; other host access remains available. Output, time and process lifetime are bounded.",
        json!({"command":{"type":"string"},"workingDirectory":{"type":"string","description":"An absolute assigned project path, or a path relative to the Sprite workspace. Defaults to the Sprite workspace."},"shell":{"type":"string","description":"Optional absolute shell executable path. Omit to use the user's configured default shell; only override when a task specifically requires another shell."},"timeoutSeconds":{"type":"integer","minimum":1,"maximum":300}}),
        &["command"],
    )]
}

pub async fn execute(args: &Value, context: &ToolContext) -> Result<ToolOutput, String> {
    if !context.allow_host_execution {
        return Err("Shell requires an explicit host-execution grant for this Sprite".into());
    }
    let explicit = args
        .get("shell")
        .map(|value| {
            value
                .as_str()
                .ok_or("shell must be an absolute executable path")
        })
        .transpose()?;
    let session = crate::shell_session::ShellSession::resolve(
        explicit,
        &context.data_directory,
        &context.workspace,
    )?;
    execute_with_session(args, context, session).await
}

async fn execute_with_session(
    args: &Value,
    context: &ToolContext,
    session: crate::shell_session::ShellSession,
) -> Result<ToolOutput, String> {
    if !context.allow_host_execution {
        return Err("Shell requires an explicit host-execution grant for this Sprite".into());
    }
    let command = required_str(args, "command")?;
    let timeout = Duration::from_secs(
        args.get("timeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(60)
            .clamp(1, 300),
    );
    let workspace = std::fs::canonicalize(&context.workspace).map_err(|e| e.to_string())?;
    let working_directory = if let Some(path) = args.get("workingDirectory") {
        let path = path
            .as_str()
            .ok_or("workingDirectory must be a string path")?;
        let path = std::path::Path::new(path);
        let candidate = if path.is_absolute() {
            path.to_owned()
        } else {
            workspace.join(path)
        };
        let candidate = std::fs::canonicalize(candidate)
            .map_err(|error| format!("Cannot open shell working directory: {error}"))?;
        let allowed = std::iter::once(&workspace)
            .chain(context.allowed_roots.iter())
            .any(|root| std::fs::canonicalize(root).is_ok_and(|root| candidate.starts_with(root)));
        if !allowed || !candidate.is_dir() {
            return Err(
                "Shell working directory must be inside the Sprite workspace or an assigned project"
                    .into(),
            );
        }
        candidate
    } else {
        workspace.clone()
    };
    let arguments = session.arguments(command, &working_directory)?;
    let mut cmd =
        crate::shell_scope::command(session.executable(), &arguments, &context.denied_roots)?;
    session.apply_environment(&mut cmd);
    cmd.current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    // Interactive startup must not take control of the launching app's terminal.
    // A new session also owns a process group for cancellation and timeout cleanup.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start shell: {e}"))?;
    let _process_group = ProcessGroup(child.id());
    let stdout = child.stdout.take().ok_or("Missing shell stdout")?;
    let stderr = child.stderr.take().ok_or("Missing shell stderr")?;
    // Readers are joined inside the same future so cancellation cannot detach them.
    let execution = async {
        let (status, stdout, stderr) = tokio::join!(
            child.wait(),
            read_bounded(stdout, context.progress.as_ref()),
            read_bounded(stderr, context.progress.as_ref()),
        );
        let status = status.map_err(|e| e.to_string())?;
        let (stdout, out_truncated) = stdout?;
        let (stderr, err_truncated) = stderr?;
        Ok(ToolOutput::json(
            json!({"exitCode":status.code(),"success":status.success(),"stdout":stdout,"stderr":stderr,"truncated":out_truncated || err_truncated}),
        ))
    };
    tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => Err("Shell cancelled; owned process group terminated".into()),
        result = tokio::time::timeout(timeout, execution) => result.map_err(|_| "Shell timed out; owned process group terminated".to_owned())?,
    }
}

pub(crate) struct ProcessGroup(pub Option<u32>);

impl ProcessGroup {
    pub fn terminate(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.take() {
            // Each launched executable owns a new process group. This also
            // terminates descendants that kept pipes open after their parent exited.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// Decode complete UTF-8 sequences only; a pipe read may split a character.
fn take_display_text(pending: &mut Vec<u8>, eof: bool) -> String {
    let mut text = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(valid) => {
                text.push_str(valid);
                pending.clear();
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                // SAFETY: valid_up_to is guaranteed by Utf8Error.
                text.push_str(std::str::from_utf8(&pending[..valid]).unwrap());
                pending.drain(..valid);
                if let Some(invalid) = error.error_len() {
                    text.push('�');
                    pending.drain(..invalid);
                } else {
                    if eof && !pending.is_empty() {
                        text.push('�');
                        pending.clear();
                    }
                    break;
                }
            }
        }
    }
    text
}

fn publish_progress(
    progress: Option<&tokio::sync::watch::Sender<ToolProgress>>,
    text: &str,
    truncated: bool,
) {
    if let Some(progress) = progress {
        progress.send_modify(|current| {
            let mut end = text
                .len()
                .min(131_072usize.saturating_sub(current.output.len()));
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            current.output.push_str(&text[..end]);
            current.truncated |= truncated || end < text.len();
        });
    }
}

async fn read_bounded(
    mut reader: impl AsyncRead + Unpin,
    progress: Option<&tokio::sync::watch::Sender<ToolProgress>>,
) -> Result<(String, bool), String> {
    let mut bytes = Vec::new();
    let mut pending = Vec::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if count == 0 {
            publish_progress(progress, &take_display_text(&mut pending, true), truncated);
            break;
        }
        let remaining = 65_536usize.saturating_sub(bytes.len());
        let accepted = count.min(remaining);
        bytes.extend_from_slice(&buffer[..accepted]);
        pending.extend_from_slice(&buffer[..accepted]);
        let was_truncated = truncated;
        truncated |= count > remaining;
        let text = take_display_text(&mut pending, truncated);
        if !text.is_empty() || truncated != was_truncated {
            publish_progress(progress, &text, truncated);
        }
        // Keep draining after the display limit so a full pipe never deadlocks a process.
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Keep tests away from the operator's real startup files and CLI logins.
    async fn execute(args: &Value, context: &ToolContext) -> Result<ToolOutput, String> {
        let session = crate::shell_session::ShellSession::for_test(
            args.get("shell")
                .and_then(Value::as_str)
                .unwrap_or("/bin/sh"),
            &context.data_directory,
            &context.workspace,
            vec![
                ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
                ("TERM".into(), "dumb".into()),
            ],
        )?;
        execute_with_session(args, context, session).await
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn unassigned_projects_are_hidden_from_shell_and_its_children() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let allowed = root.path().join("allowed");
        let denied = root.path().join("unassigned");
        for path in [&workspace, &allowed, &denied] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(denied.join("private.txt"), "excluded project contents").unwrap();
        std::fs::write(allowed.join("visible.txt"), "allowed project contents").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&denied, workspace.join("alias")).unwrap();
        let context = ToolContext {
            orbit_id: "scoped-shell".into(),
            workspace: workspace.clone(),
            data_directory: root.path().join("data"),
            allowed_roots: vec![allowed.clone()],
            denied_roots: vec![denied.clone()],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        // Values enter shell as data rather than interpolated command syntax.
        let literal =
            |path: &std::path::Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        for command in [
            format!("cat {}/private.txt", literal(&denied)),
            format!("printf changed > {}/private.txt", literal(&denied)),
            format!("cd {} && cat private.txt", literal(&denied)),
            format!(
                "/bin/sh -c 'cat \"$1/private.txt\"' sh {}",
                literal(&denied)
            ),
            "cat alias/private.txt".into(),
            "printf changed > alias/private.txt".into(),
        ] {
            let result = execute(&json!({"command":command}), &context)
                .await
                .unwrap();
            let output: Value = serde_json::from_str(&result.content).unwrap();
            assert_eq!(
                output["success"], false,
                "command bypassed exclusions: {command}"
            );
            assert!(
                !output["stdout"]
                    .as_str()
                    .unwrap()
                    .contains("excluded project contents")
            );
            assert_eq!(
                std::fs::read_to_string(denied.join("private.txt")).unwrap(),
                "excluded project contents"
            );
        }
        let result = execute(&json!({"command":"cat visible.txt; printf updated > written.txt", "workingDirectory": allowed}), &context).await.unwrap();
        let output: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["success"], true);
        assert!(
            output["stdout"]
                .as_str()
                .unwrap()
                .contains("allowed project contents")
        );
        assert_eq!(
            std::fs::read_to_string(allowed.join("written.txt")).unwrap(),
            "updated"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn shell_cannot_reopen_host_proc_or_remove_the_exclusion_mount() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let denied = root.path().join("unassigned");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&denied).unwrap();
        std::fs::write(denied.join("private.txt"), "excluded project contents").unwrap();
        let context = ToolContext {
            orbit_id: "scoped-shell".into(),
            workspace,
            data_directory: root.path().join("data"),
            allowed_roots: vec![],
            denied_roots: vec![denied.clone()],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let command = format!(
            "cat /proc/1/root{}/private.txt; umount '{}'; mv '{}' '{}-moved'",
            denied.display(),
            denied.display(),
            denied.display(),
            denied.display()
        );
        let result = execute(&json!({"command": command}), &context)
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["success"], false);
        assert!(
            !output["stdout"]
                .as_str()
                .unwrap()
                .contains("excluded project contents")
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn cancellation_terminates_scoped_shell_descendants() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let denied = root.path().join("unassigned");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&denied).unwrap();
        let context = ToolContext {
            orbit_id: "scoped-cancel".into(),
            workspace: workspace.clone(),
            data_directory: root.path().join("data"),
            allowed_roots: vec![],
            denied_roots: vec![denied],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let token = context.cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            token.cancel();
        });
        let started = std::time::Instant::now();
        assert!(
            execute(
                &json!({"command":"(sleep 1; echo survived > escaped.txt) & wait"}),
                &context
            )
            .await
            .unwrap_err()
            .contains("cancelled")
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !workspace.join("escaped.txt").exists(),
            "scoped child survived cancellation"
        );
    }

    #[tokio::test]
    async fn progress_arrives_before_command_finishes_and_survives_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let (progress, mut updates) = tokio::sync::watch::channel(ToolProgress::default());
        let context = ToolContext {
            orbit_id: "stream-test".into(),
            workspace: root.path().into(),
            data_directory: root.path().into(),
            allowed_roots: vec![],
            denied_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: Some(progress),
        };
        let args = json!({"command":"printf first; sleep 20; printf last"});
        let execution = execute(&args, &context);
        tokio::pin!(execution);
        tokio::select! {
            result = &mut execution => panic!("command finished before output: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    updates.changed().await.unwrap();
                    if updates.borrow().output.contains("first") { break; }
                }
            }) => result.unwrap(),
        }
        assert!(updates.borrow().output.contains("first"));
        context.cancellation.cancel();
        assert!(execution.await.unwrap_err().contains("cancelled"));
        assert!(!updates.borrow().output.contains("last"));
    }

    #[tokio::test]
    async fn progress_preserves_split_utf8_and_bounds_a_flood_without_blocking() {
        let (progress, updates) = tokio::sync::watch::channel(ToolProgress::default());
        let (mut writer, reader) = tokio::io::duplex(16);
        let producer = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            for byte in "🪐 é".as_bytes() {
                writer.write_all(&[*byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
            writer.write_all(&vec![b'x'; 100_000]).await.unwrap();
        });
        let (output, truncated) = read_bounded(reader, Some(&progress)).await.unwrap();
        producer.await.unwrap();
        let latest = updates.borrow();
        assert!(latest.output.starts_with("🪐 é"));
        assert_eq!(latest.output, output);
        assert!(truncated && latest.truncated);
        assert_eq!(latest.output.len(), 65_536);
    }

    #[tokio::test]
    async fn private_cli_login_is_separate_per_orbit_and_startup_restores_working_directory() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("installed-cli");
        std::fs::create_dir(&bin).unwrap();
        let glab = bin.join("glab");
        std::fs::write(&glab, "#!/bin/sh\n[ -z \"$GITLAB_TOKEN$SSH_AUTH_SOCK\" ] || exit 41\n[ \"$GLAB_USE_KEYRING\" = false ] || exit 42\n[ -f \"$GLAB_CONFIG_DIR/config.yml\" ] || exit 40\n[ \"$(cat \"$GLAB_CONFIG_DIR/config.yml\")\" = orbit-a-login ] || exit 43\nprintf 'Authenticated Sprite A CLI\\n'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let contexts: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|name| {
                let data = root.path().join(name);
                let workspace = data.join("workspace with spaces ' and $dollars");
                std::fs::create_dir_all(&workspace).unwrap();
                std::fs::write(
                    workspace.join(".bashrc"),
                    "export RC_MARKER=private\ncd /\n",
                )
                .unwrap();
                ToolContext {
                    orbit_id: name.into(),
                    data_directory: data,
                    workspace,
                    allowed_roots: vec![],
                    denied_roots: vec![],
                    allow_host_execution: true,
                    screen_share_allowed: false,
                    cancellation: Default::default(),
                    progress: None,
                }
            })
            .collect();
        let host = root.path().join("host-login");
        std::fs::create_dir(&host).unwrap();
        std::fs::write(host.join("config.yml"), "host-login-must-not-be-used").unwrap();
        let environment = vec![
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", bin.display()).into(),
            ),
            ("GLAB_CONFIG_DIR".into(), host.clone().into_os_string()),
            ("XDG_CONFIG_HOME".into(), host.clone().into_os_string()),
            ("GITLAB_TOKEN".into(), "host-token-must-not-be-used".into()),
            ("SSH_AUTH_SOCK".into(), "host-agent-must-not-be-used".into()),
            ("ZDOTDIR".into(), host.into_os_string()),
        ];
        let args = json!({"command":"glab auth status && printf '%s\\n' \"$HOME\" \"$PWD\" \"$RC_MARKER\""});
        async fn run(
            args: &Value,
            context: &ToolContext,
            environment: &[(std::ffi::OsString, std::ffi::OsString)],
        ) -> Result<ToolOutput, String> {
            let session = crate::shell_session::ShellSession::for_test(
                "/bin/bash",
                &context.data_directory,
                &context.workspace,
                environment.to_vec(),
            )
            .unwrap();
            execute_with_session(args, context, session).await
        }
        let first = run(&args, &contexts[0], &environment).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&first.content).unwrap()["exitCode"],
            40
        );
        std::fs::write(
            contexts[0]
                .data_directory
                .join("config/glab-cli/config.yml"),
            "orbit-a-login",
        )
        .unwrap();
        let logged_in = run(&args, &contexts[0], &environment).await.unwrap();
        let value: Value = serde_json::from_str(&logged_in.content).unwrap();
        assert_eq!(value["exitCode"], 0, "{value}");
        let expected = format!(
            "Authenticated Sprite A CLI\n{}\n{}\nprivate\n",
            contexts[0].workspace.display(),
            contexts[0].workspace.display()
        );
        assert!(
            value["stdout"].as_str().unwrap().ends_with(&expected),
            "{value}"
        );
        let other = run(&args, &contexts[1], &environment).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&other.content).unwrap()["exitCode"],
            40
        );
    }

    #[tokio::test]
    async fn private_zsh_startup_files_run_inside_project_exclusions() {
        if !std::path::Path::new("/bin/zsh").is_file() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("orbit");
        let workspace = data.join("workspace");
        let denied = root.path().join("unassigned");
        let host = root.path().join("host");
        for path in [&workspace, &denied, &host] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(denied.join("secret"), "must not reach startup").unwrap();
        std::fs::write(host.join(".zshrc"), "export HOST_SECRET=should-not-load\n").unwrap();
        std::fs::write(workspace.join(".zshenv"), "export PRIVATE_ENV=loaded\n").unwrap();
        std::fs::write(
            workspace.join(".zprofile"),
            "export PRIVATE_PROFILE=loaded\n",
        )
        .unwrap();
        let secret = denied
            .join("secret")
            .to_string_lossy()
            .replace('\'', "'\\''");
        std::fs::write(workspace.join(".zshrc"), format!("if cat '{secret}' 2>/dev/null; then export STARTUP_SCOPE=leaked; else export STARTUP_SCOPE=protected; fi\nexport PRIVATE_RC=loaded\ncd /\n")).unwrap();
        let context = ToolContext {
            orbit_id: "private-zsh".into(),
            data_directory: data.clone(),
            workspace: workspace.clone(),
            allowed_roots: vec![],
            denied_roots: vec![denied],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let session = crate::shell_session::ShellSession::for_test(
            "/bin/zsh",
            &data,
            &workspace,
            vec![("ZDOTDIR".into(), host.into_os_string())],
        )
        .unwrap();
        let output = execute_with_session(&json!({"command":"printf '%s|%s|%s|%s|%s|%s' \"$PRIVATE_ENV\" \"$PRIVATE_PROFILE\" \"$PRIVATE_RC\" \"$STARTUP_SCOPE\" \"$HOST_SECRET\" \"$PWD\""}), &context, session).await.unwrap();
        let value: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(value["exitCode"], 0, "{value}");
        assert_eq!(
            value["stdout"],
            format!("loaded|loaded|loaded|protected||{}", workspace.display())
        );
    }

    #[tokio::test]
    async fn grants_output_and_timeout() {
        let root = tempfile::tempdir().unwrap();
        let mut ctx = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
            data_directory: root.path().into(),
            allowed_roots: vec![],
            denied_roots: vec![],
            allow_host_execution: false,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        assert!(
            execute(&json!({"command":"printf hello"}), &ctx)
                .await
                .is_err()
        );
        ctx.allow_host_execution = true;
        assert!(
            execute(&json!({"command":"printf hello"}), &ctx)
                .await
                .unwrap()
                .content
                .contains("hello")
        );
        let started = std::time::Instant::now();
        assert!(
            execute(&json!({"command":"sleep 20","timeoutSeconds":1}), &ctx)
                .await
                .unwrap_err()
                .contains("timed out")
        );
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[tokio::test]
    async fn shell_can_start_in_an_assigned_project_but_not_an_unassigned_folder() {
        let root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let context = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
            data_directory: root.path().into(),
            allowed_roots: vec![project.path().into()],
            denied_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let result = execute(
            &json!({"command":"pwd", "workingDirectory":project.path()}),
            &context,
        )
        .await
        .unwrap();
        let result: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            result["stdout"].as_str().unwrap().trim(),
            project.path().canonicalize().unwrap().to_string_lossy()
        );
        assert!(
            execute(
                &json!({"command":"pwd", "workingDirectory":outside.path()}),
                &context
            )
            .await
            .is_err()
        );
        assert!(
            execute(&json!({"command":"pwd", "workingDirectory":42}), &context)
                .await
                .unwrap_err()
                .contains("must be a string")
        );
    }

    #[tokio::test]
    async fn cancellation_stops_an_owned_process_group() {
        let root = tempfile::tempdir().unwrap();
        let ctx = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
            data_directory: root.path().into(),
            allowed_roots: vec![],
            denied_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let token = ctx.cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });
        let started = std::time::Instant::now();
        assert!(
            execute(&json!({"command":"sleep 20 & wait"}), &ctx)
                .await
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
