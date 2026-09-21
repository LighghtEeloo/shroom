use std::{process::Stdio, time::Duration};

use shroom_integrations::{GuestDirectory, Project};
use tokio::io::AsyncReadExt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Cannot open guest directory {0}. Edit the default working directory or use the home login to repair it."
    )]
    DirectoryUnavailable(GuestDirectory),
    #[error("Checking guest directory {0} timed out. Check SSH access and try again.")]
    DirectoryCheckTimedOut(GuestDirectory),
    #[error("Could not check guest directory {directory}: {source}")]
    Io {
        directory: GuestDirectory,
        source: std::io::Error,
    },
    #[error("SSH could not check guest directory {directory}: {diagnostic}")]
    Ssh {
        directory: GuestDirectory,
        diagnostic: String,
    },
}

pub struct ProjectCheck;

impl ProjectCheck {
    pub async fn check(project: &Project) -> Result<(), Error> {
        Self::run(
            project.check_directory_command(),
            &project.directory,
            Duration::from_secs(10),
        )
        .await
    }

    async fn run(
        command: std::process::Command,
        directory: &GuestDirectory,
        timeout: Duration,
    ) -> Result<(), Error> {
        let mut child = tokio::process::Command::from(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| Error::Io {
                directory: directory.clone(),
                source,
            })?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let result = tokio::time::timeout(timeout, async {
            tokio::try_join!(child.wait(), Self::drain(stdout), Self::drain(stderr))
        })
        .await;
        let (status, _, diagnostic) = match result {
            Ok(result) => result.map_err(|source| Error::Io {
                directory: directory.clone(),
                source,
            })?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(Error::DirectoryCheckTimedOut(directory.clone()));
            }
        };
        if status.success() {
            Ok(())
        } else if status.code() == Some(Project::DIRECTORY_UNAVAILABLE) {
            Err(Error::DirectoryUnavailable(directory.clone()))
        } else {
            Err(Error::Ssh {
                directory: directory.clone(),
                diagnostic: format!("{status}: {diagnostic}"),
            })
        }
    }

    async fn drain(mut reader: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<String> {
        let mut tail = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = reader.read(&mut buffer).await?;
            if count == 0 {
                return Ok(String::from_utf8_lossy(&tail)
                    .chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                    .collect());
            }
            tail.extend_from_slice(&buffer[..count]);
            if tail.len() > buffer.len() {
                tail.drain(..tail.len() - buffer.len());
            }
        }
    }
}

#[cfg(test)]
mod tests;
