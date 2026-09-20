use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    str::FromStr,
    time::Duration,
};

use ssh_key::{Algorithm, HashAlg, PrivateKey, PublicKey};
use tokio::{io::AsyncReadExt, process::Command, time};

use crate::{
    ClientPublicKey, Error, GuestUser, HostKeyAlias, HostPublicKey, Result, SshConnection,
};

const HELPER_TIMEOUT: Duration = Duration::from_secs(10);
const FILE_LIMIT: u64 = 16 * 1024;
pub(crate) const GUEST_HELPER: &str = "/usr/local/sbin/shroom-ssh";
pub(crate) const GUEST_HOST_KEY: &str = "/etc/ssh/ssh_host_ed25519_key.pub";

impl HostPublicKey {
    pub fn to_openssh(&self) -> Result<String> {
        Ok(self.0.to_openssh()?)
    }

    pub fn alias(&self) -> HostKeyAlias {
        let digest: String = self
            .0
            .fingerprint(HashAlg::Sha256)
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        HostKeyAlias(format!("shroom-{digest}"))
    }
}

impl FromStr for HostPublicKey {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Ok(Self(Access::public_key(value)?))
    }
}

impl ClientPublicKey {
    pub fn to_openssh(&self) -> Result<String> {
        Ok(self.0.to_openssh()?)
    }
}

impl FromStr for ClientPublicKey {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Ok(Self(Access::public_key(value)?))
    }
}

pub(crate) struct Access {
    pub(crate) directory: PathBuf,
}

pub(crate) struct Material {
    pub(crate) host_key: HostPublicKey,
}

impl Access {
    fn public_key(value: &str) -> Result<PublicKey> {
        let value = value.trim();
        if value.lines().count() != 1 {
            return Err(Error::InvalidAccess("expected one public key"));
        }
        let mut key = PublicKey::from_openssh(value)?;
        if key.algorithm() != Algorithm::Ed25519 {
            return Err(Error::InvalidAccess("expected an Ed25519 key"));
        }
        key.set_comment("");
        Ok(key)
    }

    pub(crate) fn create_directory(path: &Path) -> Result<()> {
        DirBuilder::new().recursive(true).mode(0o700).create(path)?;
        Self::check_directory(path)
    }

    fn check_directory(path: &Path) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
            return Err(Error::InvalidAccess(
                "access directory must be a real directory with mode 0700",
            ));
        }
        Ok(())
    }

    pub(crate) fn exists(&self) -> Result<bool> {
        match fs::symlink_metadata(&self.directory) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) async fn create(&self) -> Result<ClientPublicKey> {
        match DirBuilder::new().mode(0o700).create(&self.directory) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::AccessConflict(self.directory.clone()));
            }
            Err(error) => return Err(error.into()),
        }
        let output = Helper::output(
            Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-C", "shroom", "-f"])
                .arg(self.directory.join("client_ed25519")),
            HELPER_TIMEOUT,
        )
        .await?;
        Helper::success("ssh-keygen", &output)?;
        let public = Self::read_file(&self.directory.join("client_ed25519.pub"), false)?
            .ok_or_else(|| Error::AccessIncomplete(self.directory.clone()))?;
        public.parse()
    }

    pub(crate) fn pin(&self, key: &HostPublicKey) -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.directory.join("known_hosts"))?;
        writeln!(file, "{} {}", key.alias(), key.to_openssh()?)?;
        file.sync_all()?;
        Ok(())
    }

    fn read_file(path: &Path, private: bool) -> Result<Option<String>> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.len() > FILE_LIMIT {
            return Err(Error::InvalidAccess("expected a small regular access file"));
        }
        if private && metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(Error::InvalidAccess(
                "client private key must have mode 0600",
            ));
        }
        let mut text = String::new();
        File::open(path)?
            .take(FILE_LIMIT + 1)
            .read_to_string(&mut text)?;
        if text.len() as u64 > FILE_LIMIT {
            return Err(Error::InvalidAccess("access file is too large"));
        }
        Ok(Some(text))
    }

    pub(crate) fn read(&self) -> Result<Option<Material>> {
        if !self.exists()? {
            return Ok(None);
        }
        Self::check_directory(&self.directory)?;
        // Validate each existing file even if another file is absent.
        let private = Self::read_file(&self.directory.join("client_ed25519"), true)?
            .map(PrivateKey::from_openssh)
            .transpose()?;
        if private
            .as_ref()
            .is_some_and(|key| key.is_encrypted() || key.algorithm() != Algorithm::Ed25519)
        {
            return Err(Error::InvalidAccess(
                "expected an unencrypted Ed25519 client key",
            ));
        }
        let public = Self::read_file(&self.directory.join("client_ed25519.pub"), false)?
            .map(|text| text.parse::<ClientPublicKey>())
            .transpose()?;
        let host_key = Self::read_file(&self.directory.join("known_hosts"), false)?
            .map(|text| Self::parse_pin(&text))
            .transpose()?;
        if let (Some(private), Some(public)) = (&private, &public)
            && private.public_key().key_data() != public.0.key_data()
        {
            return Err(Error::InvalidAccess(
                "client public and private keys disagree",
            ));
        }
        Ok(match (private, public, host_key) {
            (Some(_), Some(_), Some(host_key)) => Some(Material { host_key }),
            _ => None,
        })
    }

    fn parse_pin(text: &str) -> Result<HostPublicKey> {
        let text = text.trim();
        let (alias, key) = text
            .split_once(' ')
            .ok_or(Error::InvalidAccess("malformed known_hosts pin"))?;
        let key: HostPublicKey = key.parse()?;
        if alias != key.alias().as_str() {
            return Err(Error::InvalidAccess(
                "known_hosts alias does not match its key",
            ));
        }
        Ok(key)
    }

    pub(crate) fn connection(
        &self,
        material: Material,
        endpoint: SocketAddr,
        user: &GuestUser,
    ) -> SshConnection {
        SshConnection {
            endpoint,
            user: user.to_string(),
            identity_file: self.directory.join("client_ed25519"),
            host_key_alias: material.host_key.alias(),
            host_key: material.host_key,
            known_hosts_file: self.directory.join("known_hosts"),
            directory: user.directory(),
        }
    }

    pub(crate) fn remove(&self) -> Result<()> {
        match fs::symlink_metadata(&self.directory) {
            Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&self.directory)?,
            Ok(_) => fs::remove_file(&self.directory)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}

impl SshConnection {
    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new("ssh");
        command
            .args([
                "-F",
                "/dev/null",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "IdentitiesOnly=yes",
                "-o",
                "IdentityAgent=none",
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "CheckHostIP=no",
                "-o",
                "GlobalKnownHostsFile=/dev/null",
                "-o",
                "UpdateHostKeys=no",
                "-o",
                "PasswordAuthentication=no",
                "-o",
                "KbdInteractiveAuthentication=no",
                "-o",
                "PreferredAuthentications=publickey",
                "-o",
                "HostKeyAlgorithms=ssh-ed25519",
                "-o",
                "ConnectTimeout=3",
                "-o",
                "ConnectionAttempts=1",
                "-o",
                "ClearAllForwardings=yes",
            ])
            .arg("-o")
            .arg(format!("HostKeyAlias={}", self.host_key_alias))
            .arg("-o")
            .arg(format!(
                "UserKnownHostsFile={}",
                Self::ssh_path(&self.known_hosts_file)
            ))
            .arg("-o")
            .arg(format!(
                "IdentityFile={}",
                Self::ssh_path(&self.identity_file)
            ))
            .arg("-p")
            .arg(self.endpoint.port().to_string())
            .arg("-l")
            .arg(&self.user)
            .arg(self.endpoint.ip().to_string());
        command
    }

    fn ssh_path(path: &Path) -> String {
        // OpenSSH parses -o values and expands percent tokens even without a shell.
        format!(
            "\"{}\"",
            path.to_string_lossy()
                .replace('%', "%%")
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        )
    }

    pub(crate) async fn probe(&self) -> Result<()> {
        let deadline = time::Instant::now() + Duration::from_secs(15);
        loop {
            let output =
                Helper::output(self.command().arg("id -un"), Duration::from_secs(5)).await?;
            if output.status.success() && output.stdout == format!("{}\n", self.user).as_bytes() {
                return Ok(());
            }
            let diagnostic = Helper::diagnostic(&output.stderr);
            if diagnostic.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
                || diagnostic.contains("Host key verification failed")
            {
                return Err(Error::HostKeyMismatch);
            }
            if time::Instant::now() >= deadline {
                return Err(Error::SshUnavailable(diagnostic));
            }
            time::sleep(Duration::from_millis(200)).await;
        }
    }
}

#[cfg(test)]
mod tests;

pub(crate) struct Helper;

impl Helper {
    pub(crate) async fn output(command: &mut Command, timeout: Duration) -> Result<Output> {
        let mut child = command
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("LC_ALL", "C")
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env_remove("SSH_AUTH_SOCK")
            .env_remove("SSH_ASKPASS")
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (status, stdout, stderr) = time::timeout(timeout, async {
            tokio::try_join!(child.wait(), Self::read(stdout), Self::read(stderr))
        })
        .await??;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    async fn read(stream: impl tokio::io::AsyncRead + Unpin) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        stream.take(FILE_LIMIT).read_to_end(&mut bytes).await?;
        Ok(bytes)
    }

    pub(crate) fn diagnostic(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes)
            .chars()
            .take(1024)
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect()
    }

    pub(crate) fn success(program: &'static str, output: &Output) -> Result<()> {
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::HelperFailed {
                program,
                code: output.status.code(),
                diagnostic: Self::diagnostic(&output.stderr),
            })
        }
    }

    pub(crate) async fn prerequisites() -> Result<()> {
        let output = Self::output(Command::new("ssh").arg("-V"), HELPER_TIMEOUT).await?;
        Self::success("ssh", &output)?;
        // -? is read-only and intentionally exits unsuccessfully after printing usage.
        let _ = Self::output(Command::new("ssh-keygen").arg("-?"), HELPER_TIMEOUT).await?;
        Ok(())
    }
}
