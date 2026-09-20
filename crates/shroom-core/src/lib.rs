//! SSH-first workspaces over the local microsandbox SDK.
//!
//! [`Core`] owns a state-directory lock, not the lifetime of its detached workspaces.
//! Refresh connection details with [`Core::get`] before reconnecting after a restart.

mod access;
mod core;
mod options;

use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use derive_more::Display;
use ssh_key::PublicKey;

pub use crate::core::Core;
pub use microsandbox::{MicrosandboxError, sandbox::SandboxStatus};
pub use options::{FolderAccess, GuestUser, HostFolder, WorkspaceOptions};

/// The preinstalled runtime version paired with the pinned SDK revision.
pub const RUNTIME_VERSION: &str = "0.7.2";
/// Build `images/workspace/` and import this tag into the core's private SDK home.
pub const WORKSPACE_IMAGE: &str = "localhost/shroom-workspace:0.1.0";

pub type Result<T> = std::result::Result<T, Error>;

/// Paths to the matching preinstalled runtime pair. No downloader is invoked.
#[derive(Clone, Debug)]
pub struct Config {
    /// The actual executable, rather than a shell wrapper around it.
    pub runtime_executable: PathBuf,
    pub firmware: PathBuf,
}

/// A lowercase ASCII slug, safe as one access-directory component.
#[derive(Clone, Debug, Display, Eq, Hash, PartialEq)]
pub struct WorkspaceName(String);

impl WorkspaceName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for WorkspaceName {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        if !(1..=48).contains(&value.len())
            || !value.bytes().next().is_some_and(alphanumeric)
            || !value.bytes().all(|b| alphanumeric(b) || b == b'-')
        {
            return Err(Error::InvalidName);
        }
        Ok(Self(value.to_owned()))
    }
}

/// An explicitly selected unprivileged host port (1024–65535).
#[derive(Clone, Copy, Debug, Display, Eq, PartialEq)]
pub struct HostPort(u16);

impl HostPort {
    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u32> for HostPort {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        if !(1024..=65535).contains(&value) {
            return Err(Error::InvalidPort(value));
        }
        Ok(Self(value as u16))
    }
}

/// A validated Ed25519 guest identity, obtained through SDK administration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostPublicKey(PublicKey);

/// A validated Ed25519 client identity. The private half stays on the host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientPublicKey(PublicKey);

/// A safe, endpoint-independent identifier derived from the host key's SHA-256 fingerprint.
#[derive(Clone, Debug, Display, Eq, PartialEq)]
pub struct HostKeyAlias(String);

impl HostKeyAlias {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub name: WorkspaceName,
    pub state: SandboxStatus,
    pub options: WorkspaceOptions,
    /// Present for a running VM with an endpoint and complete access files.
    /// Only `create` and `start` perform an authenticated readiness probe.
    pub ssh: Option<SshConnection>,
}

#[derive(Clone, Debug)]
pub struct SshConnection {
    pub endpoint: SocketAddr,
    pub user: String,
    pub identity_file: PathBuf,
    pub host_key: HostPublicKey,
    pub host_key_alias: HostKeyAlias,
    pub known_hosts_file: PathBuf,
    /// Client hint; an ordinary SSH session starts in the account's home.
    pub directory: String,
}

/// The failed boundary of an operation; partial artifacts are left for explicit removal.
#[derive(Clone, Copy, Debug, Display, Eq, PartialEq)]
pub enum Stage {
    Validate,
    ReadCatalog,
    ReadAccess,
    CreateAccess,
    CreateSandbox,
    Provision,
    PinHostKey,
    Start,
    DiscoverEndpoint,
    Probe,
    Stop,
    RemoveSandbox,
    RemoveAccess,
}

impl Stage {
    fn context(self, name: &WorkspaceName, source: impl Into<Error>) -> Error {
        Error::Operation {
            name: name.clone(),
            stage: self,
            source: Box::new(source.into()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "guest username must be 1–32 lowercase letters, digits, underscores or hyphens, start with a letter, and not be root"
    )]
    InvalidGuestUser,
    #[error("{0}")]
    InvalidHostFolder(&'static str),
    #[error(
        "workspace name must be a 1–48 character lowercase ASCII slug starting with a letter or digit"
    )]
    InvalidName,
    #[error("host port {0} must be in 1024–65535")]
    InvalidPort(u32),
    #[error("another core holds the state-directory lock: {0}")]
    CoreInUse(PathBuf),
    #[error("access directory already exists: {0}")]
    AccessConflict(PathBuf),
    #[error("access files are missing or incomplete: {0}")]
    AccessIncomplete(PathBuf),
    #[error("invalid access material: {0}")]
    InvalidAccess(&'static str),
    #[error("port {port} is already recorded for workspace {workspace}")]
    PortInUse {
        port: HostPort,
        workspace: WorkspaceName,
    },
    #[error("SSH host key does not match the stored pin")]
    HostKeyMismatch,
    #[error("SSH is unavailable: {0}")]
    SshUnavailable(String),
    #[error("{program} failed (exit {code:?}): {diagnostic}")]
    HelperFailed {
        program: &'static str,
        code: Option<i32>,
        diagnostic: String,
    },
    #[error("guest SSH helper failed (exit {code}): {diagnostic}")]
    GuestHelperFailed { code: i32, diagnostic: String },
    #[error("{name}: {stage}: {source}")]
    Operation {
        name: WorkspaceName,
        stage: Stage,
        #[source]
        source: Box<Error>,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Key(#[from] ssh_key::Error),
    #[error(transparent)]
    Deadline(#[from] tokio::time::error::Elapsed),
    #[error(transparent)]
    Sdk(#[from] MicrosandboxError),
}
