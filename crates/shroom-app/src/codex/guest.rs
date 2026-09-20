use std::{process::Stdio, time::Duration};

use shroom_integrations::{Attachment, RemoteCommand, Terminal};
use tokio::io::AsyncReadExt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Could not prepare Codex in the workspace: {0}")]
    Io(#[from] std::io::Error),
    #[error("Codex setup in the workspace timed out. Check its internet connection and try again.")]
    Timeout,
    #[error("Codex setup in the workspace failed: {0}")]
    Setup(String),
}

pub(super) struct Guest;

impl Guest {
    const INSTALL: &str = include_str!("install.sh");
    const VERIFY: &str = r#"
set -eu
if ! command -v codex >/dev/null 2>&1; then
    echo 'Codex is not on the guest login-shell PATH. Add $HOME/.local/bin to PATH in the guest login profile and try again.' >&2
    exit 1
fi
exec codex --version
"#;

    pub(super) async fn prepare(attachment: &Attachment) -> Result<(), Error> {
        Self::run(
            Self::command(attachment, Self::INSTALL),
            Duration::from_secs(300),
        )
        .await?;
        // Use another SSH login: an installer can succeed while changing only its own PATH.
        Self::run(
            Self::command(attachment, Self::VERIFY),
            Duration::from_secs(30),
        )
        .await
    }

    fn command(attachment: &Attachment, script: &str) -> std::process::Command {
        let remote = RemoteCommand::new("/bin/sh")
            .and_then(|command| command.with_arg("-c"))
            .and_then(|command| command.with_arg(script))
            .expect("fixed guest setup script");
        attachment.command(&remote, Terminal::None)
    }

    async fn run(command: std::process::Command, timeout: Duration) -> Result<(), Error> {
        let mut child = tokio::process::Command::from(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let result = tokio::time::timeout(timeout, async {
            tokio::try_join!(child.wait(), Self::tail(stdout), Self::tail(stderr))
        })
        .await;
        let (status, stdout, stderr) = match result {
            Ok(result) => result?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(Error::Timeout);
            }
        };
        if status.success() {
            return Ok(());
        }
        let diagnostic = if stderr.is_empty() { stdout } else { stderr };
        let diagnostic = String::from_utf8_lossy(&diagnostic)
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect::<String>();
        Err(Error::Setup(format!("{status}: {}", diagnostic.trim())))
    }

    async fn tail(mut reader: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
        const LIMIT: usize = 4096;
        let mut output = Vec::new();
        let mut buffer = [0; LIMIT];
        loop {
            let count = reader.read(&mut buffer).await?;
            if count == 0 {
                return Ok(output);
            }
            output.extend_from_slice(&buffer[..count]);
            if output.len() > LIMIT {
                output.drain(..output.len() - LIMIT);
            }
        }
    }
}

#[cfg(test)]
mod tests;
