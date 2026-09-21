use std::process::Command;

use crate::{GuestPort, Project, RemoteCommand, Terminal};

impl Project {
    /// Register this concrete SSH alias and select its remote folder in the Codex desktop app.
    /// Install the SSH stanza first. This route was inspected in desktop version 26.915.31945;
    /// it is not a documented public API, and opening it does not prove connection success.
    pub fn codex_project_url(&self) -> url::Url {
        let mut url = url::Url::parse("codex://settings/connections/ssh/add")
            .expect("fixed Codex connection route");
        url.query_pairs_mut()
            .append_pair("name", self.attachment.alias().as_str())
            .append_pair("projectPath", self.directory.as_str())
            .append_pair("enabled", "true");
        url
    }
}

/// Native applications whose documented remote workflow consumes SSH connection information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeApp {
    Codex,
    ClaudeDesktop,
    Cursor,
    ZCode,
}

/// A configuration route, not a claim of end-to-end application compatibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostKeyHandling {
    /// Keep the complete exported stanza and verify the application's effective SSH options.
    SshConfig,
    /// The app only imports address/user/key fields; verify its own pinning before connecting.
    ClientVerificationRequired,
}

impl NativeApp {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Cursor => "Cursor",
            Self::ZCode => "ZCode",
        }
    }

    pub fn host_key_handling(self) -> HostKeyHandling {
        match self {
            Self::ZCode => HostKeyHandling::ClientVerificationRequired,
            _ => HostKeyHandling::SshConfig,
        }
    }

    pub fn documentation(self) -> &'static str {
        match self {
            Self::Codex => {
                "https://learn.chatgpt.com/docs/remote-connections#connect-to-an-ssh-host"
            }
            Self::ClaudeDesktop => "https://code.claude.com/docs/en/desktop#ssh-sessions",
            Self::Cursor => "https://cursor.com/docs/agent/agents-window",
            Self::ZCode => "https://zcode.z.ai/en/docs/remote-development",
        }
    }

    pub fn setup(self) -> &'static str {
        match self {
            Self::Codex => {
                "Add the complete concrete host stanza to ~/.ssh/config. Install and authenticate Codex in the guest, ensure codex is on the guest login-shell PATH, then select the SSH host and remote project folder in Connections."
            }
            Self::ClaudeDesktop => {
                "Add the complete host stanza to ~/.ssh/config, then add an SSH connection using its alias in Claude Desktop. Desktop installs Claude Code remotely on first connection; choose the guest project directory."
            }
            Self::Cursor => {
                "Use the complete host stanza with Cursor's Remote SSH workflow, let Cursor install its guest server, then open the guest project directory. Verify the effective host-key options for the installed client version."
            }
            Self::ZCode => {
                "Enter the real endpoint, username, and identity file. Alias import only prefills these fields. Verify that ZCode can enforce the supplied public host key before use; Shroom has not validated that path. Refresh the fields after each workspace restart."
            }
        }
    }
}

/// Web applications installed and authenticated by the workspace user.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebApp {
    Kimi,
    DeepSeekHarness,
}

impl WebApp {
    pub fn name(self) -> &'static str {
        match self {
            Self::Kimi => "Kimi",
            Self::DeepSeekHarness => "DeepSeek Harness",
        }
    }

    pub fn executable(self) -> &'static str {
        match self {
            Self::Kimi => "kimi",
            Self::DeepSeekHarness => "dsh",
        }
    }

    pub fn documentation(self) -> &'static str {
        match self {
            Self::Kimi => "https://github.com/MoonshotAI/kimi-cli",
            Self::DeepSeekHarness => "https://github.com/deepseek-ai/deepseek-harness",
        }
    }

    pub fn default_port(self) -> GuestPort {
        GuestPort::try_from(match self {
            Self::Kimi => 5494,
            Self::DeepSeekHarness => 3080,
        })
        .expect("fixed unprivileged ports")
    }

    /// Build a command for a preinstalled executable, without an installer or credential copying.
    /// Read the actual URL from its output before building a tunnel; Kimi may change ports.
    pub fn launch_command(self, project: &Project, requested_port: GuestPort) -> Command {
        let command = match self {
            // The caller needs the reported port while the Python web server is still running.
            Self::Kimi => RemoteCommand::new("/usr/bin/env")
                .and_then(|command| command.with_arg("PYTHONUNBUFFERED=1"))
                .and_then(|command| command.with_arg("kimi")),
            Self::DeepSeekHarness => RemoteCommand::new("dsh"),
        };
        let remote = [
            "web",
            "--no-open",
            "--host",
            "127.0.0.1",
            "--port",
            &requested_port.to_string(),
        ]
        .into_iter()
        .try_fold(command.expect("fixed program"), |command, arg| {
            command.with_arg(arg)
        })
        .expect("fixed arguments");
        project.command(&remote, Terminal::None)
    }
}
