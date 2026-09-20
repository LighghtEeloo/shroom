use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub state_dir: PathBuf,
    pub config: Config,
}

impl SessionInfo {
    pub fn runtime_home(&self) -> PathBuf {
        self.state_dir.join("microsandbox")
    }

    pub fn inspection_command(&self) -> String {
        let home = self.runtime_home();
        format!(
            "MSB_HOME={} \\\nMSB_CONFIG_PATH={} \\\n  {} list",
            Self::shell_word(&home.to_string_lossy()),
            Self::shell_word(&home.join("config.json").to_string_lossy()),
            Self::shell_word(&self.config.runtime_executable.to_string_lossy()),
        )
    }

    fn shell_word(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExportFormat {
    #[default]
    Command,
    Config,
}

impl ExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Command => "Command",
            Self::Config => "SSH config",
        }
    }

    pub fn copy_label(self) -> &'static str {
        match self {
            Self::Command => "Copy command",
            Self::Config => "Copy SSH config",
        }
    }

    pub fn copied(self) -> &'static str {
        match self {
            Self::Command => "SSH command copied",
            Self::Config => "SSH config copied. Place it before matching Host defaults.",
        }
    }
}

pub enum Outcome {
    Complete,
    Verified(Box<SshConnection>),
    Exported(Box<ConnectionExport>),
}

#[derive(Clone)]
pub struct ConnectionExport {
    pub name: WorkspaceName,
    pub format: ExportFormat,
    pub text: String,
    pub connection: SshConnection,
}

impl ConnectionExport {
    pub fn with_connection(
        name: &WorkspaceName,
        connection: SshConnection,
        format: ExportFormat,
    ) -> Result<Self> {
        let attachment = Attachment::new(format!("shroom-{name}").parse()?, connection.clone())?;
        Ok(Self {
            name: name.clone(),
            text: match format {
                ExportFormat::Command => attachment.ssh_command_line(),
                ExportFormat::Config => attachment.ssh_config(),
            },
            format,
            connection,
        })
    }

    pub fn matches(connection: &SshConnection, workspace: &Workspace) -> bool {
        workspace.state == SandboxStatus::Running
            && workspace.ssh.as_ref().is_some_and(|current| {
                current.endpoint == connection.endpoint
                    && current.user == connection.user
                    && current.identity_file == connection.identity_file
                    && current.known_hosts_file == connection.known_hosts_file
                    && current.host_key == connection.host_key
                    && current.host_key_alias == connection.host_key_alias
                    && current.directory == connection.directory
            })
    }
}

#[derive(Clone)]
pub enum CheckResult {
    Verified(Box<SshConnection>),
    Failed,
}

#[derive(Clone)]
pub struct AccessCheck {
    pub name: WorkspaceName,
    pub at: SystemTime,
    pub result: CheckResult,
}

#[derive(Clone)]
pub struct Activity {
    pub command: Command,
    pub at: SystemTime,
    pub error: Option<String>,
}

pub struct Timestamp;

impl Timestamp {
    pub fn label(at: SystemTime) -> String {
        let seconds = at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        format!(
            "{:02}:{:02}:{:02} UTC",
            seconds / 3600 % 24,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

#[derive(Clone, Default)]
pub struct Model {
    pub session: Option<SessionInfo>,
    pub workspaces: Vec<Workspace>,
    pub incomplete: Vec<WorkspaceName>,
    pub image: ImageStatus,
    pub catalog_current: bool,
    pub selected: Option<WorkspaceName>,
    pub pending: Option<Command>,
    pub confirm_remove: Option<WorkspaceName>,
    pub errors: Vec<String>,
    pub notice: Option<String>,
    pub copied: Option<ConnectionExport>,
    pub checks: Vec<AccessCheck>,
    pub activity: Vec<Activity>,
}

impl Model {
    pub fn connected(&self) -> bool {
        self.session.is_some()
    }

    pub fn begin(&mut self, command: &Command) -> bool {
        if self.pending.is_some() {
            return false;
        }
        if matches!(
            command,
            Command::Create(..)
                | Command::Start(_)
                | Command::Verify(_)
                | Command::Stop(_)
                | Command::Remove(_)
                | Command::Cleanup(_)
        ) {
            self.checks
                .retain(|check| Some(&check.name) != command.target());
        }
        self.pending = Some(command.clone());
        self.confirm_remove = None;
        self.errors.clear();
        self.notice = None;
        self.copied = None;
        true
    }

    pub fn apply(&mut self, reply: Reply) {
        let pending = self.pending.take();
        let now = SystemTime::now();
        let operation_error = reply.outcome.as_ref().err().map(ToString::to_string);
        self.errors.clear();
        self.notice = None;
        self.copied = None;
        let changed_directory = self.session.as_ref().map(|s| &s.state_dir)
            != reply.session.as_ref().map(|s| &s.state_dir);
        if changed_directory {
            self.checks.clear();
            self.activity.clear();
            self.selected = None;
            self.confirm_remove = None;
        }
        self.session = reply.session;
        self.selected = reply.selection.or(self.selected.take());
        match reply.outcome {
            Ok(outcome) => {
                match outcome {
                    Outcome::Complete => (),
                    Outcome::Verified(connection) => {
                        if let Some(name) = pending.as_ref().and_then(Command::target) {
                            self.checks.retain(|check| &check.name != name);
                            self.checks.push(AccessCheck {
                                name: name.clone(),
                                at: now,
                                result: CheckResult::Verified(connection),
                            });
                        }
                    }
                    Outcome::Exported(export) => self.copied = Some(*export),
                }
                self.notice = pending.as_ref().map(|command| command.success().to_owned());
            }
            Err(error) => {
                self.errors.push(error.to_string());
                if let Some(
                    command @ (Command::Create(..) | Command::Start(_) | Command::Verify(_)),
                ) = pending.as_ref()
                {
                    let name = command.target().expect("workspace command");
                    self.checks.retain(|check| &check.name != name);
                    self.checks.push(AccessCheck {
                        name: name.clone(),
                        at: now,
                        result: CheckResult::Failed,
                    });
                }
            }
        }
        match reply.catalog {
            Ok(Catalog {
                mut workspaces,
                mut incomplete,
            }) => {
                workspaces.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
                incomplete.sort_by(|a, b| a.as_str().cmp(b.as_str()));
                self.workspaces = workspaces;
                self.incomplete = incomplete;
                self.catalog_current = true;
                if !self
                    .workspaces
                    .iter()
                    .any(|w| Some(&w.name) == self.selected.as_ref())
                    && !self
                        .incomplete
                        .iter()
                        .any(|name| Some(name) == self.selected.as_ref())
                {
                    self.selected = self
                        .workspaces
                        .first()
                        .map(|w| w.name.clone())
                        .or_else(|| self.incomplete.first().cloned());
                }
                self.checks.retain(|check| {
                    self.workspaces.iter().any(|workspace| {
                        workspace.name == check.name
                            && workspace.state == SandboxStatus::Running
                            && match &check.result {
                                CheckResult::Verified(connection) => {
                                    ConnectionExport::matches(connection, workspace)
                                }
                                CheckResult::Failed => true,
                            }
                    })
                });
                if self.copied.as_ref().is_some_and(|export| {
                    !self.workspaces.iter().any(|workspace| {
                        workspace.name == export.name
                            && ConnectionExport::matches(&export.connection, workspace)
                    })
                }) {
                    self.copied = None;
                    self.errors
                        .push("Connection details changed. Refresh and copy again.".into());
                }
            }
            Err(error) => {
                self.catalog_current = false;
                self.copied = None;
                self.checks.clear();
                self.errors.push(format!(
                    "Could not refresh workspaces: {error}. Refresh to try again."
                ));
            }
        }
        match reply.image {
            Ok(status) => self.image = status,
            Err(error) => {
                self.image = ImageStatus::Unknown;
                self.errors
                    .push(format!("Could not check the workspace image: {error}"));
            }
        }
        if !self.connected() {
            self.copied = None;
            self.checks.clear();
        } else if let Some(command) = pending {
            let error = operation_error
                .or_else(|| (!self.errors.is_empty()).then(|| self.errors.join("\n")));
            self.activity.insert(
                0,
                Activity {
                    command,
                    at: now,
                    error,
                },
            );
            self.activity.truncate(32);
        }
    }

    pub fn worker_failed(&mut self, error: String) {
        if let Some(command) = self.pending.take() {
            self.activity.insert(
                0,
                Activity {
                    command,
                    at: SystemTime::now(),
                    error: Some(error.clone()),
                },
            );
            self.activity.truncate(32);
        }
        self.catalog_current = false;
        self.copied = None;
        self.checks.clear();
        self.errors.push(error);
    }

    pub fn select(&mut self, name: WorkspaceName) {
        self.selected = Some(name);
        self.confirm_remove = None;
        self.copied = None;
        self.notice = None;
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| Some(&w.name) == self.selected.as_ref())
    }

    pub fn selected_incomplete(&self) -> Option<&WorkspaceName> {
        self.incomplete
            .iter()
            .find(|name| Some(*name) == self.selected.as_ref())
    }

    pub fn check(&self, name: &WorkspaceName) -> Option<&AccessCheck> {
        self.checks.iter().find(|check| &check.name == name)
    }

    pub fn available(&self) -> bool {
        self.connected() && self.catalog_current && self.pending.is_none()
    }
}
