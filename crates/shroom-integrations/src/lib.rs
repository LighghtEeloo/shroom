#![doc = include_str!("../../../docs/references/agent-integrations.md")]

mod apps;
mod ssh;
mod web;

use std::{path::PathBuf, str::FromStr};

use derive_more::Display;
use shroom_core::SshConnection;

pub use apps::{HostKeyHandling, NativeApp, WebApp};
pub use ssh::{RemoteCommand, Terminal};
pub use web::{GuestPort, GuestWebUrl, WebTunnel};

pub type Result<T> = std::result::Result<T, Error>;

/// A concrete SSH config alias, selected by the caller to be unique across state roots.
#[derive(Clone, Debug, Display, Eq, PartialEq)]
pub struct SshAlias(String);

impl SshAlias {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SshAlias {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if !(1..=128).contains(&value.len())
            || !value.starts_with(|c: char| c.is_ascii_alphanumeric())
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(Error::InvalidAlias);
        }
        Ok(Self(value.to_owned()))
    }
}

/// A validated snapshot of one workspace's SSH access, not a health or readiness assertion.
#[derive(Clone, Debug)]
pub struct Attachment {
    alias: SshAlias,
    connection: SshConnection,
}

impl Attachment {
    pub fn new(alias: SshAlias, connection: SshConnection) -> Result<Self> {
        if !connection.endpoint.ip().is_loopback() || connection.endpoint.port() == 0 {
            return Err(Error::InvalidConnection("expected a loopback SSH endpoint"));
        }
        if connection.user.is_empty()
            || !connection
                .user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || connection.user.starts_with('-')
        {
            return Err(Error::InvalidConnection("invalid SSH username"));
        }
        if connection.host_key.alias() != connection.host_key_alias {
            return Err(Error::InvalidConnection("host key and alias disagree"));
        }
        ssh::Encoding::path(&connection.identity_file)?;
        ssh::Encoding::path(&connection.known_hosts_file)?;
        if !connection.directory.starts_with('/')
            || connection.directory.chars().any(char::is_control)
        {
            return Err(Error::InvalidConnection(
                "expected an absolute guest directory without control characters",
            ));
        }
        Ok(Self { alias, connection })
    }

    pub fn alias(&self) -> &SshAlias {
        &self.alias
    }

    /// Includes the original public host key for clients that implement their own SSH stack.
    pub fn connection(&self) -> &SshConnection {
        &self.connection
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "SSH alias must be 1–128 ASCII letters, digits, dots, underscores, or hyphens, starting with a letter or digit"
    )]
    InvalidAlias,
    #[error("invalid SSH connection: {0}")]
    InvalidConnection(&'static str),
    #[error("SSH path must be absolute UTF-8 without control characters or '$': {0}")]
    InvalidPath(PathBuf),
    #[error("remote program must be a command name or an absolute executable path")]
    InvalidProgram,
    #[error("remote command arguments cannot contain NUL")]
    InvalidArgument,
    #[error("guest web port {0} must be in 1024–65535")]
    InvalidGuestPort(u32),
    #[error(
        "guest URL must use HTTP, a loopback host, and an explicit unprivileged port, without credentials or control characters"
    )]
    InvalidWebUrl,
}
