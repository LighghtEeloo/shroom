use std::{env, fs, path::PathBuf};

use microsandbox::{Image, LocalBackend, MicrosandboxError};
use shroom_core::{
    Config, Core, FolderAccess, HostFolder, HostPort, SandboxStatus, SshConnection,
    WORKSPACE_IMAGE, Workspace, WorkspaceName, WorkspaceOptions,
};
use shroom_integrations::{Attachment, GuestPort, GuestWebUrl, WebApp};
use tokio::sync::Mutex;

use crate::agents::AgentManager;

mod state;
pub use state::*;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} must be an absolute path without control characters or '$'")]
    InvalidPath(&'static str),
    #[error("SSH port must be a whole number from 1024 to 65535")]
    InvalidPort,
    #[error("Open a workspace directory first")]
    Disconnected,
    #[error("A workspace directory is already open")]
    AlreadyConnected,
    #[error("SSH details are unavailable. Start the workspace and try again.")]
    SshUnavailable,
    #[error("Import the prepared workspace image before creating a workspace.")]
    ImageMissing,
    #[error("Workspace image architecture {0:?} does not match this host")]
    ImageArchitecture(Option<String>),
    #[error("Workspace {0} now exists. Refresh and use its workspace controls instead.")]
    WorkspaceExists(WorkspaceName),
    #[error("Web ports must be whole numbers from 1024 to 65535")]
    InvalidWebPort,
    #[error(transparent)]
    Agent(#[from] crate::agents::Error),
    #[error(transparent)]
    Codex(#[from] crate::codex::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sdk(#[from] MicrosandboxError),
    #[error(transparent)]
    Core(#[from] shroom_core::Error),
    #[error(transparent)]
    Integration(#[from] shroom_integrations::Error),
}

#[derive(Clone)]
pub struct SetupForm {
    pub state_dir: String,
    pub runtime: String,
    pub firmware: String,
}

impl Default for SetupForm {
    fn default() -> Self {
        let pair = PathBuf::from("/opt/homebrew/opt/microsandbox/libexec");
        Self {
            state_dir: env::var("SHROOM_STATE_DIR").unwrap_or_else(|_| {
                env::var("HOME")
                    .map(|home| format!("{home}/.shroom"))
                    .unwrap_or_default()
            }),
            runtime: Self::default_path("SHROOM_RUNTIME", pair.join("msb")),
            firmware: Self::default_path("SHROOM_FIRMWARE", pair.join("libkrunfw.5.dylib")),
        }
    }
}

impl SetupForm {
    fn default_path(variable: &str, candidate: PathBuf) -> String {
        env::var(variable).unwrap_or_else(|_| {
            if candidate.is_file() {
                candidate.to_string_lossy().into_owned()
            } else {
                String::new()
            }
        })
    }

    pub fn parse(&self) -> Result<Command> {
        Ok(Command::Open {
            state_dir: Self::path(&self.state_dir, "State directory")?,
            config: Config {
                runtime_executable: Self::path(&self.runtime, "Runtime executable")?,
                firmware: Self::path(&self.firmware, "Firmware")?,
            },
        })
    }

    pub fn path(text: &str, field: &'static str) -> Result<PathBuf> {
        let path = PathBuf::from(text);
        if !path.is_absolute() || text.chars().any(|c| c.is_control() || c == '$') {
            return Err(Error::InvalidPath(field));
        }
        Ok(path)
    }
}

#[derive(Clone)]
pub struct CreateForm {
    pub name: String,
    pub port: String,
    pub user: String,
    pub folders: Vec<FolderForm>,
}

#[derive(Clone, Default)]
pub struct FolderForm {
    pub host: String,
    pub guest: String,
    pub writable: bool,
}

impl Default for CreateForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            port: "2222".into(),
            user: "developer".into(),
            folders: Vec::new(),
        }
    }
}

impl CreateForm {
    pub fn add_folder(&mut self) {
        let guest = (1..)
            .map(|number| format!("/mnt/shared-{number}"))
            .find(|guest| self.folders.iter().all(|folder| &folder.guest != guest))
            .expect("finite folder list");
        self.folders.push(FolderForm {
            guest,
            ..Default::default()
        });
    }

    pub fn parse(&self) -> Result<Command> {
        let name = self.name.parse()?;
        let port = self.port.parse::<u32>().map_err(|_| Error::InvalidPort)?;
        let options = WorkspaceOptions {
            user: self.user.parse()?,
            folders: self
                .folders
                .iter()
                .map(|folder| {
                    HostFolder::new(
                        folder.host.clone().into(),
                        folder.guest.clone(),
                        if folder.writable {
                            FolderAccess::ReadWrite
                        } else {
                            FolderAccess::ReadOnly
                        },
                    )
                })
                .collect::<shroom_core::Result<_>>()?,
        };
        Ok(Command::Create(name, port.try_into()?, options))
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Open { state_dir: PathBuf, config: Config },
    Disconnect,
    Refresh,
    ImportImage(PathBuf),
    Create(WorkspaceName, HostPort, WorkspaceOptions),
    Start(WorkspaceName),
    Verify(WorkspaceName),
    Stop(WorkspaceName),
    Remove(WorkspaceName),
    Cleanup(WorkspaceName),
    Export(WorkspaceName, ExportFormat),
    AddToCodex(WorkspaceName),
    LaunchWeb(WorkspaceName, WebApp, GuestPort),
    ForwardWeb(WorkspaceName, WebApp, WebForward),
    CloseWeb(WorkspaceName, WebApp),
}

#[derive(Clone)]
pub struct WebForward {
    pub guest: GuestWebUrl,
    pub local_port: HostPort,
}

// A reported URL may contain a login token; diagnostics must not serialize it.
impl std::fmt::Debug for WebForward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebForward")
            .field("local_port", &self.local_port)
            .finish_non_exhaustive()
    }
}

impl WebForward {
    pub fn parse(url: &str, port: &str) -> Result<Self> {
        Ok(Self {
            guest: url.parse()?,
            local_port: port
                .parse::<u32>()
                .map_err(|_| Error::InvalidWebPort)?
                .try_into()?,
        })
    }
}

impl Command {
    pub fn progress(&self) -> &'static str {
        match self {
            Self::Open { .. } => "Opening workspace directory…",
            Self::Disconnect => "Closing workspace directory…",
            Self::Refresh => "Refreshing workspaces…",
            Self::ImportImage(_) => "Importing workspace image…",
            Self::Create(..) => "Creating workspace and verifying SSH…",
            Self::Start(_) => "Starting workspace and verifying SSH…",
            Self::Verify(_) => "Verifying SSH access…",
            Self::Stop(_) => "Stopping workspace…",
            Self::Remove(_) => "Removing workspace…",
            Self::Cleanup(_) => "Removing incomplete setup…",
            Self::Export(..) => "Reading current SSH details…",
            Self::AddToCodex(_) => "Preparing Codex in workspace and opening project…",
            Self::LaunchWeb(..) => "Launching web app through SSH…",
            Self::ForwardWeb(..) => "Opening tunnel and checking web app…",
            Self::CloseWeb(..) => "Closing web connection…",
        }
    }

    fn selection(&self) -> Option<WorkspaceName> {
        match self {
            Self::Create(name, ..)
            | Self::Start(name)
            | Self::Verify(name)
            | Self::Stop(name)
            | Self::Export(name, _)
            | Self::AddToCodex(name)
            | Self::LaunchWeb(name, ..)
            | Self::ForwardWeb(name, ..)
            | Self::CloseWeb(name, _) => Some(name.clone()),
            _ => None,
        }
    }

    pub fn target(&self) -> Option<&WorkspaceName> {
        match self {
            Self::Create(name, ..)
            | Self::Start(name)
            | Self::Verify(name)
            | Self::Stop(name)
            | Self::Remove(name)
            | Self::Cleanup(name)
            | Self::Export(name, _)
            | Self::AddToCodex(name)
            | Self::LaunchWeb(name, ..)
            | Self::ForwardWeb(name, ..)
            | Self::CloseWeb(name, _) => Some(name),
            _ => None,
        }
    }

    pub fn success(&self) -> &'static str {
        match self {
            Self::Open { .. } => "Workspace directory opened",
            Self::Disconnect => "Workspace directory closed",
            Self::Refresh => "Workspace information refreshed",
            Self::ImportImage(_) => "Workspace image imported",
            Self::Create(..) => "Workspace created; SSH verified",
            Self::Start(_) => "Workspace started; SSH verified",
            Self::Verify(_) => "SSH access verified",
            Self::Stop(_) => "Workspace stopped; files preserved",
            Self::Remove(_) => "Workspace deleted",
            Self::Cleanup(_) => "Incomplete setup removed",
            Self::Export(..) => "Current SSH details read",
            Self::AddToCodex(_) => {
                "Codex CLI ready; project sent to Codex. Finish any sign-in or connection setup there."
            }
            Self::LaunchWeb(..) => "Web launch started. Read its output for the actual URL.",
            Self::ForwardWeb(..) => "Web tunnel checked",
            Self::CloseWeb(..) => "SSH launch and tunnel closed. The workspace is still running.",
        }
    }

    async fn execute(self, slot: &mut Option<Session>, agents: &AgentManager) -> Result<Outcome> {
        match self {
            Self::Open { state_dir, config } => {
                if slot.is_some() {
                    return Err(Error::AlreadyConnected);
                }
                *slot = Some(Session::open(state_dir, config).await?);
                Ok(Outcome::Complete)
            }
            Self::Disconnect => {
                agents.close(None).await;
                *slot = None;
                Ok(Outcome::Complete)
            }
            command => {
                let session = slot.as_mut().ok_or(Error::Disconnected)?;
                match command {
                    Self::Refresh => (),
                    Self::ImportImage(archive) => {
                        Image::load_local(&session.images, &archive, vec![WORKSPACE_IMAGE.into()])
                            .await?;
                        session.require_image().await?;
                    }
                    Self::Create(name, port, options) => {
                        // Check before Core creates access files, keeping a missing-image retry clean.
                        session.require_image().await?;
                        let workspace = session.core.create(name, port, options).await?;
                        return Ok(Outcome::Verified(Box::new(
                            workspace.ssh.ok_or(Error::SshUnavailable)?,
                        )));
                    }
                    Self::Verify(name) => {
                        if session.core.get(&name).await?.state != SandboxStatus::Running {
                            return Err(Error::SshUnavailable);
                        }
                        let workspace = session.core.start(&name).await?;
                        return Ok(Outcome::Verified(Box::new(
                            workspace.ssh.ok_or(Error::SshUnavailable)?,
                        )));
                    }
                    Self::Start(name) => {
                        let workspace = session.core.start(&name).await?;
                        return Ok(Outcome::Verified(Box::new(
                            workspace.ssh.ok_or(Error::SshUnavailable)?,
                        )));
                    }
                    Self::Stop(name) => {
                        agents.close(Some((&name, None))).await;
                        session.core.stop(&name).await?;
                    }
                    Self::Remove(name) => {
                        agents.close(Some((&name, None))).await;
                        session.core.remove(&name).await?;
                    }
                    Self::Cleanup(name) => session.cleanup(&name).await?,
                    Self::Export(name, format) => {
                        // Always fetch again: list snapshots can outlive an endpoint.
                        let connection = session
                            .core
                            .get(&name)
                            .await?
                            .ssh
                            .ok_or(Error::SshUnavailable)?;
                        return Ok(Outcome::Exported(Box::new(
                            ConnectionExport::with_connection(&name, connection, format)?,
                        )));
                    }
                    Self::AddToCodex(name) => {
                        // Resolve current runtime details before touching the user's SSH config.
                        let attachment = session.attachment(&name).await?;
                        crate::codex::Codex::add(&name, attachment.connection().clone()).await?;
                    }
                    Self::LaunchWeb(name, app, port) => {
                        let attachment = session.attachment(&name).await?;
                        agents.launch(name, app, attachment, port).await?;
                    }
                    Self::ForwardWeb(name, app, forward) => {
                        let attachment = session.attachment(&name).await?;
                        agents
                            .forward(&name, app, attachment, forward.guest, forward.local_port)
                            .await?;
                    }
                    Self::CloseWeb(name, app) => agents.close(Some((&name, Some(app)))).await,
                    Self::Open { .. } | Self::Disconnect => unreachable!(),
                }
                Ok(Outcome::Complete)
            }
        }
    }
}

struct Session {
    core: Core,
    images: LocalBackend,
    state_dir: PathBuf,
    config: Config,
}

impl Session {
    async fn attachment(&self, name: &WorkspaceName) -> Result<Attachment> {
        let workspace = self.core.get(name).await?;
        if workspace.state != SandboxStatus::Running {
            return Err(Error::SshUnavailable);
        }
        Ok(Attachment::new(
            format!("shroom-{name}").parse()?,
            workspace.ssh.ok_or(Error::SshUnavailable)?,
        )?)
    }

    async fn open(state_dir: PathBuf, config: Config) -> Result<Self> {
        let core = Core::open(state_dir.clone(), config.clone()).await?;
        let state_dir = state_dir.canonicalize()?;
        let home = state_dir.join("microsandbox");
        // Core has validated and written this root's SDK configuration. Image setup belongs to the app.
        let images = LocalBackend::builder()
            .home(&home)
            .config_path(home.join("config.json"))
            .build_lazy()?;
        Ok(Self {
            core,
            images,
            state_dir,
            config,
        })
    }

    async fn image_status(&self) -> Result<ImageStatus> {
        match Image::get_local(&self.images, WORKSPACE_IMAGE).await {
            Ok(image) => {
                Self::check_architecture(image.architecture())?;
                Ok(ImageStatus::Ready)
            }
            Err(MicrosandboxError::ImageNotFound(_)) => Ok(ImageStatus::Missing),
            Err(error) => Err(error.into()),
        }
    }

    fn check_architecture(architecture: Option<&str>) -> Result<()> {
        let expected = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "amd64",
            other => other,
        };
        if architecture != Some(expected) {
            return Err(Error::ImageArchitecture(architecture.map(str::to_owned)));
        }
        Ok(())
    }

    async fn require_image(&self) -> Result<()> {
        match self.image_status().await? {
            ImageStatus::Ready => Ok(()),
            _ => Err(Error::ImageMissing),
        }
    }

    async fn cleanup(&mut self, name: &WorkspaceName) -> Result<()> {
        match self.core.get(name).await {
            Ok(_) => Err(Error::WorkspaceExists(name.clone())),
            Err(shroom_core::Error::Operation {
                stage: shroom_core::Stage::ReadCatalog,
                source,
                ..
            }) if matches!(
                *source,
                shroom_core::Error::Sdk(MicrosandboxError::SandboxNotFound(_))
            ) =>
            {
                self.core.remove(name).await?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn catalog(&self) -> Result<Catalog> {
        let workspaces = self.core.list().await?;
        let incomplete = fs::read_dir(self.state_dir.join("access"))?
            .map(|entry| {
                let entry = entry?;
                Ok(entry
                    .file_type()?
                    .is_dir()
                    .then(|| {
                        entry
                            .file_name()
                            .to_str()
                            .and_then(|name| name.parse::<WorkspaceName>().ok())
                    })
                    .flatten())
            })
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .filter(|name| !workspaces.iter().any(|workspace| &workspace.name == name))
            .collect();
        Ok(Catalog {
            workspaces,
            incomplete,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageStatus {
    #[default]
    Unknown,
    Missing,
    Ready,
}

#[derive(Default)]
pub struct Catalog {
    pub workspaces: Vec<Workspace>,
    pub incomplete: Vec<WorkspaceName>,
}

/// The mutex owns the single core and serializes operations off the Freya UI thread.
#[derive(Default)]
pub struct Backend {
    session: Mutex<Option<Session>>,
    pub agents: AgentManager,
}

pub struct Reply {
    pub session: Option<SessionInfo>,
    pub selection: Option<WorkspaceName>,
    pub outcome: Result<Outcome>,
    pub catalog: Result<Catalog>,
    pub image: Result<ImageStatus>,
}

impl Backend {
    pub async fn execute(&self, command: Command) -> Reply {
        let mut session = self.session.lock().await;
        let selection = command.selection();
        let outcome = command.execute(&mut session, &self.agents).await;
        // Failures may leave native artifacts, so refresh even when the operation fails.
        let (catalog, image) = match session.as_ref() {
            Some(session) => (session.catalog().await, session.image_status().await),
            None => (Ok(Catalog::default()), Ok(ImageStatus::Unknown)),
        };
        match &catalog {
            Ok(catalog) => self.agents.reconcile(&catalog.workspaces).await,
            Err(_) => self.agents.close(None).await,
        }
        Reply {
            session: session.as_ref().map(|session| SessionInfo {
                state_dir: session.state_dir.clone(),
                config: session.config.clone(),
            }),
            selection,
            outcome,
            catalog,
            image,
        }
    }
}

pub struct Actions;

impl Actions {
    pub fn start(state: SandboxStatus) -> bool {
        matches!(
            state,
            SandboxStatus::Created
                | SandboxStatus::Stopped
                | SandboxStatus::Crashed
                | SandboxStatus::Running
        )
    }

    pub fn stop(state: SandboxStatus) -> bool {
        matches!(state, SandboxStatus::Running | SandboxStatus::Paused)
    }

    pub fn remove(state: SandboxStatus) -> bool {
        matches!(
            state,
            SandboxStatus::Created | SandboxStatus::Stopped | SandboxStatus::Crashed
        )
    }
}

#[cfg(test)]
mod tests;
