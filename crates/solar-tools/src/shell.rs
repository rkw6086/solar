use crate::{
    ToolContext, ToolDefinition, ToolOutput, definition, managed_environment, required_str,
};
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![definition(
        "shell_run",
        "Run a shell command after host execution is approved. Optional workingDirectory must be the Orbit workspace or an assigned project folder. The command itself can access the host beyond file-tool folders. Output, time and process lifetime are bounded.",
        json!({"command":{"type":"string"},"workingDirectory":{"type":"string","description":"An absolute assigned project path, or a path relative to the Orbit workspace. Defaults to the Orbit workspace."},"shell":{"type":"string","enum":["/bin/sh","/bin/bash","/bin/zsh"]},"timeoutSeconds":{"type":"integer","minimum":1,"maximum":300}}),
        &["command"],
    )]
}

pub async fn execute(args: &Value, context: &ToolContext) -> Result<ToolOutput, String> {
    if !context.allow_host_execution {
        return Err("Shell requires an explicit host-execution grant for this Orbit".into());
    }
    let command = required_str(args, "command")?;
    let shell = args
        .get("shell")
        .and_then(Value::as_str)
        .unwrap_or("/bin/sh");
    if !["/bin/sh", "/bin/bash", "/bin/zsh"].contains(&shell) {
        return Err("Unsupported shell".into());
    }
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
                "Shell working directory must be inside the Orbit workspace or an assigned project"
                    .into(),
            );
        }
        candidate
    } else {
        workspace.clone()
    };
    let environment = managed_environment(&context.data_directory)?;
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(working_directory)
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        )
        .envs(environment)
        .env("HOME", &workspace)
        .env("LANG", "en_US.UTF-8")
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start shell: {e}"))?;
    let _process_group = ProcessGroup(child.id());
    let stdout = child.stdout.take().ok_or("Missing shell stdout")?;
    let stderr = child.stderr.take().ok_or("Missing shell stderr")?;
    // Readers are joined inside the same future so cancellation cannot detach them.
    let execution = async {
        let (status, stdout, stderr) =
            tokio::join!(child.wait(), read_bounded(stdout), read_bounded(stderr));
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

async fn read_bounded(mut reader: impl AsyncRead + Unpin) -> Result<(String, bool), String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        let remaining = 65_536usize.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shell_uses_orbit_storage_for_preferences_cache_and_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let context = ToolContext {
            orbit_id: "test".into(),
            data_directory: root.path().into(),
            workspace: workspace.clone(),
            allowed_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
        };
        let output = execute(
            &json!({"command":"printf '%s\\n' \"$HOME\" \"$XDG_CACHE_HOME\" \"$XDG_CONFIG_HOME\" \"$XDG_DATA_HOME\" \"$XDG_STATE_HOME\" \"$TMPDIR\""}),
            &context,
        ).await.unwrap();
        let value: Value = serde_json::from_str(&output.content).unwrap();
        let paths: Vec<_> = value["stdout"].as_str().unwrap().lines().collect();
        assert_eq!(std::path::Path::new(paths[0]), workspace);
        for path in &paths[1..] {
            let path = std::path::Path::new(path);
            assert!(path.starts_with(root.path()));
            assert!(path.is_dir());
        }
    }

    #[tokio::test]
    async fn grants_output_and_timeout() {
        let root = tempfile::tempdir().unwrap();
        let mut ctx = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
            data_directory: root.path().into(),
            allowed_roots: vec![],
            allow_host_execution: false,
            screen_share_allowed: false,
            cancellation: Default::default(),
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
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
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
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
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
