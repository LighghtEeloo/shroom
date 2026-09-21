use std::{process::Command, str::FromStr};

use derive_more::Display;

use crate::{Attachment, Error, RemoteCommand, Result, Terminal, ssh::Encoding};

/// A literal absolute path inside the guest, independent of the host filesystem.
#[derive(Clone, Debug, Display, Eq, PartialEq)]
pub struct GuestDirectory(String);

impl GuestDirectory {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for GuestDirectory {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if !value.starts_with('/')
            || value.chars().any(char::is_control)
            || value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}') != value
        {
            return Err(Error::InvalidGuestDirectory);
        }
        Ok(Self(value.into()))
    }
}

/// One selected directory on an immutable SSH attachment; changing a preference creates a new snapshot.
#[derive(Clone, Debug)]
pub struct Project {
    pub attachment: Attachment,
    pub directory: GuestDirectory,
}

impl Project {
    /// The fixed directory guard failed, before invoking the requested program.
    pub const DIRECTORY_UNAVAILABLE: i32 = 72;

    fn guard(&self) -> String {
        format!(
            "if ! cd -- {} || ! test -r . || ! test -x .; then printf '%s\\n' {} >&2; exit {}; fi",
            Encoding::shell_word(self.directory.as_str()),
            Encoding::shell_word(&format!(
                "Cannot open guest directory {}. Edit the default working directory or use the home login to repair it.",
                self.directory,
            )),
            Self::DIRECTORY_UNAVAILABLE,
        )
    }

    /// Check and enter the selected directory before executing a literal program and arguments.
    pub fn command(&self, remote: &RemoteCommand, terminal: Terminal) -> Command {
        self.attachment
            .script_command(&format!("{}; {}", self.guard(), remote.script()), terminal)
    }

    /// An unstarted, authenticated directory check, executed as the workspace user.
    pub fn check_directory_command(&self) -> Command {
        self.attachment
            .script_command(&format!("{}; exit 0", self.guard()), Terminal::None)
    }

    pub fn terminal_command(&self) -> Command {
        self.command(
            &RemoteCommand::new("/bin/bash")
                .and_then(|remote| remote.with_arg("-i"))
                .expect("fixed shell"),
            Terminal::Interactive,
        )
    }

    /// Render the same argv as terminal_command for a POSIX host shell.
    pub fn terminal_command_line(&self) -> String {
        let command = self.terminal_command();
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| Encoding::shell_word(arg.to_str().expect("validated SSH arguments")))
            .collect::<Vec<_>>()
            .join(" \\\n  ")
    }

    pub fn kimi_command(&self) -> Command {
        self.command(
            &RemoteCommand::new("kimi").expect("fixed program"),
            Terminal::Interactive,
        )
    }
}
