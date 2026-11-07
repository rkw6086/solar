use crate::{ToolContext, ToolDefinition, ToolOutput, definition, required_str};
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![definition(
        "shell_run",
        "Run a shell command after an explicit host-execution grant. This can access the host beyond approved file-tool folders. Output, time and process lifetime are bounded.",
        json!({"command":{"type":"string"},"shell":{"type":"string","enum":["/bin/sh","/bin/bash","/bin/zsh"]},"timeoutSeconds":{"type":"integer","minimum":1,"maximum":300}}),
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
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(&workspace)
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        )
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
    async fn grants_output_and_timeout() {
        let root = tempfile::tempdir().unwrap();
        let mut ctx = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
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
    async fn cancellation_stops_an_owned_process_group() {
        let root = tempfile::tempdir().unwrap();
        let ctx = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
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
