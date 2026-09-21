use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::project::ProjectCheck;
use shroom_core::{SshConnection, WorkspaceName};
use shroom_integrations::{Attachment, Project};
use tempfile::NamedTempFile;

mod guest;

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("An absolute HOME directory is required to register Codex's SSH connection")]
    HomeUnavailable,
    #[error("SSH config is not a regular file. Use the SSH config tab for manual setup.")]
    ConfigNotRegular,
    #[error(
        "Shroom's SSH config markers are incomplete or duplicated. Use manual setup until they are repaired."
    )]
    InvalidManagedEntry,
    #[error("The Codex SSH alias already exists outside Shroom's managed entry. Use manual setup.")]
    AliasConflict,
    #[error("Another Shroom instance is updating SSH config. Try again.")]
    ConfigBusy,
    #[error("SSH config changed during registration. No replacement was written; try again.")]
    ConfigChanged,
    #[error("OpenSSH could not resolve the proposed config. Existing SSH config was preserved.")]
    InvalidSshConfig,
    #[error(
        "SSH defaults change this workspace's connection, identities, commands, or forwards. Existing config was preserved; use manual setup."
    )]
    ConflictingSshOptions,
    #[error("OpenSSH config verification timed out. Existing SSH config was preserved.")]
    ProbeTimeout,
    #[error(
        "SSH entry saved, but Codex could not be opened. Open Codex and use Connections, or try again: {0}"
    )]
    Launch(std::io::Error),
    #[error("Codex SSH registration failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Integration(#[from] shroom_integrations::Error),
    #[error("Codex registration worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Project(#[from] crate::project::Error),
    #[error(transparent)]
    Guest(#[from] guest::Error),
}

pub struct Codex;

impl Codex {
    pub fn attachment(name: &WorkspaceName, connection: SshConnection) -> Result<Attachment> {
        // The full host identity distinguishes equal names in different state roots and recreations.
        let alias = format!("{name}-{}", connection.host_key_alias).parse()?;
        Ok(Attachment::new(alias, connection)?)
    }

    pub async fn add(name: &WorkspaceName, mut project: Project) -> Result<()> {
        project.attachment = Self::attachment(name, project.attachment.connection().clone())?;
        let url = project.codex_project_url();
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(Error::HomeUnavailable)?;
        Self::prepare_project(&project).await?;
        Self::register(home.join(".ssh"), project.attachment).await?;
        tokio::task::spawn_blocking(move || open::that(url.as_str()))
            .await?
            .map_err(Error::Launch)
    }

    pub(crate) async fn prepare_project(project: &Project) -> Result<()> {
        ProjectCheck::check(project).await?;
        guest::Guest::prepare(&project.attachment).await?;
        ProjectCheck::check(project).await?;
        Ok(())
    }

    async fn register(directory: PathBuf, attachment: Attachment) -> Result<()> {
        let alias = attachment.alias().to_string();
        let prepared =
            tokio::task::spawn_blocking(move || PreparedConfig::new(&directory, &attachment))
                .await??;
        let expected = Self::resolve(prepared.isolated.path(), &alias).await?;
        let actual = Self::resolve(prepared.probe.path(), &alias).await?;
        if expected != actual {
            return Err(Error::ConflictingSshOptions);
        }
        tokio::task::spawn_blocking(move || prepared.commit()).await?
    }

    async fn resolve(config: &Path, alias: &str) -> Result<Vec<String>> {
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new("ssh")
                .args(["-G", "-F"])
                .arg(config)
                .arg(alias)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| Error::ProbeTimeout)??;
        if !output.status.success() {
            return Err(Error::InvalidSshConfig);
        }
        // OpenSSH's normalized text is compared at the serialization boundary. Include additive
        // identities/forwards and inherited commands as well as every exported connection option.
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| {
                matches!(
                    line.split_once(' ').map(|(key, _)| key),
                    Some(
                        "hostname"
                            | "port"
                            | "user"
                            | "batchmode"
                            | "identitiesonly"
                            | "identityagent"
                            | "addkeystoagent"
                            | "stricthostkeychecking"
                            | "checkhostip"
                            | "globalknownhostsfile"
                            | "updatehostkeys"
                            | "passwordauthentication"
                            | "kbdinteractiveauthentication"
                            | "preferredauthentications"
                            | "hostkeyalgorithms"
                            | "forwardagent"
                            | "forwardx11"
                            | "permitlocalcommand"
                            | "controlmaster"
                            | "controlpath"
                            | "proxycommand"
                            | "proxyjump"
                            | "connecttimeout"
                            | "connectionattempts"
                            | "serveraliveinterval"
                            | "serveralivecountmax"
                            | "hostkeyalias"
                            | "identityfile"
                            | "userknownhostsfile"
                            | "certificatefile"
                            | "localforward"
                            | "remoteforward"
                            | "dynamicforward"
                            | "remotecommand"
                            | "localcommand"
                            | "knownhostscommand"
                            | "canonicalizehostname"
                    )
                )
            })
            .map(str::to_owned)
            .collect())
    }
}

struct ConfigLock(File);

impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

struct PreparedConfig {
    path: PathBuf,
    original: Option<String>,
    updated: String,
    staged: NamedTempFile,
    isolated: NamedTempFile,
    probe: NamedTempFile,
    _lock: ConfigLock,
}

impl PreparedConfig {
    fn new(directory: &Path, attachment: &Attachment) -> Result<Self> {
        match fs::DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        let lock_path = directory.join(".shroom-codex.lock");
        Self::require_regular(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path)?;
        match lock.try_lock() {
            Ok(()) => (),
            Err(fs::TryLockError::WouldBlock) => return Err(Error::ConfigBusy),
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let lock = ConfigLock(lock);
        let path = directory.join("config");
        let original = Self::read(&path)?;
        let updated = Self::merge(original.as_deref().unwrap_or_default(), attachment)?;
        let staged = Self::temporary(directory, &updated)?;
        let isolated = Self::temporary(directory, &attachment.ssh_config())?;
        // -F skips the system file. Include it explicitly when checking the candidate so system
        // defaults that add identities or forwards are also rejected before installing the entry.
        let probe = Self::temporary(
            directory,
            &format!("{updated}\nHost *\nInclude /etc/ssh/ssh_config\n"),
        )?;
        Ok(Self {
            path,
            original,
            updated,
            staged,
            isolated,
            probe,
            _lock: lock,
        })
    }

    fn merge(original: &str, attachment: &Attachment) -> Result<String> {
        let alias = attachment.alias().as_str();
        let begin = format!("# BEGIN SHROOM CODEX {alias}\n");
        let end = format!("# END SHROOM CODEX {alias}\n");
        let remaining = match (
            original.matches(&begin).count(),
            original.matches(&end).count(),
        ) {
            (0, 0) => original.to_owned(),
            (1, 1) => {
                let start = original.find(&begin).expect("one begin marker");
                let finish = original.find(&end).expect("one end marker");
                if start > finish || (start != 0 && !original[..start].ends_with('\n')) {
                    return Err(Error::InvalidManagedEntry);
                }
                let body = &original[start + begin.len()..finish];
                if !body.starts_with(&format!("Host {alias}\n"))
                    || !body.ends_with("Host *\n")
                    || body
                        .lines()
                        .skip(1)
                        .any(|line| !line.starts_with("    ") && line != "Host *")
                {
                    return Err(Error::InvalidManagedEntry);
                }
                format!("{}{}", &original[..start], &original[finish + end.len()..])
            }
            _ => return Err(Error::InvalidManagedEntry),
        };
        // Reject damaged markers too, including a truncated last line, instead of adding a duplicate.
        if remaining.contains(begin.trim_end()) || remaining.contains(end.trim_end()) {
            return Err(Error::InvalidManagedEntry);
        }
        if remaining.lines().any(|line| {
            let mut words = line
                .split([' ', '\t', '=', '"', '\''])
                .filter(|word| !word.is_empty());
            words
                .next()
                .is_some_and(|word| word.eq_ignore_ascii_case("Host"))
                && words
                    .take_while(|word| !word.starts_with('#'))
                    .any(|word| word.eq_ignore_ascii_case(alias))
        }) {
            return Err(Error::AliasConflict);
        }
        // Reset Host scope before the untouched text, including any leading global directives.
        Ok(format!(
            "{begin}{}Host *\n{end}{remaining}",
            attachment.ssh_config()
        ))
    }

    fn require_regular(path: &Path) -> Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => Ok(()),
            Ok(_) => Err(Error::ConfigNotRegular),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn read(path: &Path) -> Result<Option<String>> {
        Self::require_regular(path)?;
        match fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn temporary(directory: &Path, text: &str) -> Result<NamedTempFile> {
        let mut file = NamedTempFile::new_in(directory)?;
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        Ok(file)
    }

    fn commit(self) -> Result<()> {
        if Self::read(&self.path)? != self.original {
            return Err(Error::ConfigChanged);
        }
        if self.original.as_deref() == Some(&self.updated) {
            return Ok(());
        }
        if let Some(original) = &self.original {
            let mut backup = tempfile::Builder::new()
                .prefix("config.shroom-backup-")
                .tempfile_in(self.path.parent().expect("SSH directory"))?;
            backup.write_all(original.as_bytes())?;
            backup.as_file().sync_all()?;
            backup.keep().map_err(|error| error.error)?;
            self.staged
                .as_file()
                .set_permissions(fs::metadata(&self.path)?.permissions())?;
        }
        if Self::read(&self.path)? != self.original {
            return Err(Error::ConfigChanged);
        }
        if self.original.is_none() {
            self.staged.persist_noclobber(&self.path).map_err(|error| {
                if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                    Error::ConfigChanged
                } else {
                    Error::Io(error.error)
                }
            })?;
        } else {
            self.staged
                .persist(&self.path)
                .map_err(|error| error.error)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
