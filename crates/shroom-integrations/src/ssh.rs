use std::{path::Path, process::Command};

use crate::{Attachment, Error, Result};

/// Whether a remote command needs an interactive terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Terminal {
    Interactive,
    None,
}

/// An executable and literal arguments. Shell syntax is never inferred from an argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteCommand {
    program: String,
    args: Vec<String>,
}

impl RemoteCommand {
    pub fn new(program: impl Into<String>) -> Result<Self> {
        let program = program.into();
        if program.is_empty()
            || program.starts_with(['-', '.'])
            || (!program.starts_with('/') && program.contains('/'))
            || !program
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-.+".contains(&b))
        {
            return Err(Error::InvalidProgram);
        }
        Ok(Self {
            program,
            args: Vec::new(),
        })
    }

    pub fn with_arg(mut self, arg: impl Into<String>) -> Result<Self> {
        let arg = arg.into();
        if arg.contains('\0') {
            return Err(Error::InvalidArgument);
        }
        self.args.push(arg);
        Ok(self)
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub(crate) fn script(&self) -> String {
        let command = std::iter::once(&self.program)
            .chain(&self.args)
            .map(|arg| Encoding::shell_word(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("exec {command}")
    }
}

impl Attachment {
    /// Render one concrete host stanza. Put it before matching defaults in the chosen SSH config.
    /// No RemoteCommand or forwarding directives are imposed on native applications.
    pub fn ssh_config(&self) -> String {
        let options = self
            .options()
            .into_iter()
            .map(|(key, value)| format!("    {key} {value}\n"))
            .collect::<String>();
        format!("Host {}\n{options}", self.alias)
    }

    /// Render a POSIX-shell command for an interactive login with the same pinned trust policy.
    /// It is independent of the user's SSH configuration and never changes that configuration.
    pub fn ssh_command_line(&self) -> String {
        let options = self
            .options()
            .into_iter()
            .map(|(key, value)| {
                format!(
                    "  -o {} \\\n",
                    Encoding::shell_word(&format!("{key}={value}"))
                )
            })
            .collect::<String>();
        format!(
            "ssh -F /dev/null \\\n{options}  {}",
            Encoding::shell_word(self.alias.as_str())
        )
    }

    /// Run account-level setup in the guest login environment without selecting a project.
    pub fn login_command(&self, remote: &RemoteCommand, terminal: Terminal) -> Command {
        self.script_command(&remote.script(), terminal)
    }

    pub(crate) fn script_command(&self, script: &str, terminal: Terminal) -> Command {
        let mut command = self.ssh_command();
        command
            .arg(match terminal {
                Terminal::Interactive => "-tt",
                Terminal::None => "-T",
            })
            .args(["-o", "ClearAllForwardings=yes"])
            .arg(self.alias.as_str())
            .arg(format!(
                "exec /bin/bash -lc {}",
                Encoding::shell_word(script)
            ));
        command
    }

    pub(crate) fn ssh_command(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args(["-F", "/dev/null"]);
        for (key, value) in self.options() {
            command.arg("-o").arg(format!("{key}={value}"));
        }
        command
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env_remove("SSH_AUTH_SOCK")
            .env_remove("SSH_ASKPASS");
        command
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        let connection = &self.connection;
        // Text pairs are the OpenSSH serialization boundary, shared by both renderers.
        [
            ("BatchMode", "yes"),
            ("IdentitiesOnly", "yes"),
            ("IdentityAgent", "none"),
            ("AddKeysToAgent", "no"),
            ("StrictHostKeyChecking", "yes"),
            ("CheckHostIP", "no"),
            ("GlobalKnownHostsFile", "/dev/null"),
            ("UpdateHostKeys", "no"),
            ("PasswordAuthentication", "no"),
            ("KbdInteractiveAuthentication", "no"),
            ("PreferredAuthentications", "publickey"),
            ("HostKeyAlgorithms", "ssh-ed25519"),
            ("ForwardAgent", "no"),
            ("ForwardX11", "no"),
            ("PermitLocalCommand", "no"),
            ("ControlMaster", "no"),
            ("ControlPath", "none"),
            ("ProxyCommand", "none"),
            ("ConnectTimeout", "10"),
            ("ConnectionAttempts", "1"),
            ("ServerAliveInterval", "15"),
            ("ServerAliveCountMax", "3"),
        ]
        .into_iter()
        .map(|(key, value)| (key, value.to_owned()))
        .chain([
            ("HostName", connection.endpoint.ip().to_string()),
            ("Port", connection.endpoint.port().to_string()),
            ("User", connection.user.clone()),
            ("HostKeyAlias", connection.host_key_alias.to_string()),
            (
                "IdentityFile",
                Encoding::path(&connection.identity_file).expect("validated attachment"),
            ),
            (
                "UserKnownHostsFile",
                Encoding::path(&connection.known_hosts_file).expect("validated attachment"),
            ),
        ])
        .collect()
    }
}

pub(crate) struct Encoding;

impl Encoding {
    pub(crate) fn path(path: &Path) -> Result<String> {
        let text = path
            .to_str()
            .filter(|text| path.is_absolute() && !text.chars().any(|c| c.is_control() || c == '$'))
            .ok_or_else(|| Error::InvalidPath(path.to_owned()))?;
        // OpenSSH expands percent tokens and parses quotes even in a single -o argv value.
        Ok(format!(
            "\"{}\"",
            text.replace('%', "%%")
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        ))
    }

    pub(crate) fn shell_word(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
