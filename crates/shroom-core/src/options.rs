use std::{
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use derive_more::Display;
use microsandbox::{
    SandboxConfig,
    sandbox::{HostPermissions, StatVirtualization, VolumeMount},
};

use crate::{Error, Result};

/// A non-root Linux login name. The guest keeps UID/GID 1000 for every name.
#[derive(Clone, Debug, Display, Eq, PartialEq)]
pub struct GuestUser(String);

impl Default for GuestUser {
    fn default() -> Self {
        Self("developer".into())
    }
}

impl GuestUser {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn directory(&self) -> String {
        format!("/home/{self}/workspace")
    }
}

impl FromStr for GuestUser {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if !(1..=32).contains(&value.len())
            || !value.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            || value == "root"
        {
            return Err(Error::InvalidGuestUser);
        }
        Ok(Self(value.into()))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FolderAccess {
    #[default]
    ReadOnly,
    ReadWrite,
}

/// An explicitly selected host directory, mounted below /mnt in the guest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFolder {
    host: PathBuf,
    guest: String,
    access: FolderAccess,
}

impl HostFolder {
    pub fn new(host: PathBuf, guest: String, access: FolderAccess) -> Result<Self> {
        Self::validate_paths(&host, &guest)?;
        if !host.is_dir() {
            return Err(Error::InvalidHostFolder(
                "host folder must be an existing directory",
            ));
        }
        let host = host.canonicalize()?;
        Self::validate_paths(&host, &guest)?;
        Ok(Self {
            host,
            guest,
            access,
        })
    }

    pub fn host(&self) -> &Path {
        &self.host
    }

    pub fn guest(&self) -> &str {
        &self.guest
    }

    pub fn access(&self) -> FolderAccess {
        self.access
    }

    fn validate_paths(host: &Path, guest: &str) -> Result<()> {
        if !host.is_absolute()
            || host
                .to_str()
                .is_none_or(|s| s.chars().any(char::is_control))
        {
            return Err(Error::InvalidHostFolder(
                "host folder must be an absolute UTF-8 path without control characters",
            ));
        }
        // A separate subtree keeps mounts away from the account and SSH provisioning paths.
        if !guest.starts_with("/mnt/")
            || guest.ends_with('/')
            || guest
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == "..")
            || guest.chars().any(char::is_control)
            || Path::new(guest)
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(Error::InvalidHostFolder(
                "guest folder must be a normalized path below /mnt, such as /mnt/project",
            ));
        }
        Ok(())
    }

    fn from_mount(mount: &VolumeMount) -> Result<Self> {
        let VolumeMount::Bind {
            host,
            guest,
            options,
            follow_root_symlinks,
            stat_virtualization,
            host_permissions,
            quota_mib,
        } = mount
        else {
            return Err(Error::InvalidHostFolder(
                "only host directory mounts are supported",
            ));
        };
        Self::validate_paths(host, guest)?;
        if *follow_root_symlinks
            || *stat_virtualization != StatVirtualization::Strict
            || *host_permissions != HostPermissions::Private
            || quota_mib.is_some()
            || options.noexec
            || !options.nosuid
            || !options.nodev
            || options.override_uid != Some(1000)
            || options.override_gid != Some(1000)
        {
            return Err(Error::InvalidHostFolder(
                "host folder settings conflict with the guest profile",
            ));
        }
        Ok(Self {
            host: host.clone(),
            guest: guest.clone(),
            access: if options.readonly {
                FolderAccess::ReadOnly
            } else {
                FolderAccess::ReadWrite
            },
        })
    }
}

/// Immutable creation choices persisted in microsandbox's catalog.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceOptions {
    pub user: GuestUser,
    pub folders: Vec<HostFolder>,
}

impl WorkspaceOptions {
    pub(crate) const USER_LABEL: &str = "shroom.user";

    pub(crate) fn from_config(config: &SandboxConfig) -> Result<Self> {
        let user = config
            .spec
            .labels
            .get(Self::USER_LABEL)
            .map(|user| user.parse())
            .transpose()?
            .unwrap_or_default();
        let options = Self {
            user,
            folders: config
                .spec
                .mounts
                .iter()
                .map(HostFolder::from_mount)
                .collect::<Result<_>>()?,
        };
        if !matches!(config.spec.runtime.workdir.as_deref(), Some("/"))
            && config.spec.runtime.workdir.as_deref() != Some(options.user.directory().as_str())
        {
            return Err(Error::InvalidHostFolder(
                "guest working directory does not match its account",
            ));
        }
        options.validate()?;
        Ok(options)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for (index, folder) in self.folders.iter().enumerate() {
            if self.folders[..index].iter().any(|other| {
                Path::new(folder.guest()).starts_with(other.guest())
                    || Path::new(other.guest()).starts_with(folder.guest())
            }) {
                return Err(Error::InvalidHostFolder("guest folders must not overlap"));
            }
        }
        Ok(())
    }

    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.user == other.user
            && self.folders.len() == other.folders.len()
            && self
                .folders
                .iter()
                .all(|folder| other.folders.contains(folder))
    }

    pub(crate) fn validate_sources(&self, state: &Path) -> Result<()> {
        self.validate()?;
        let state = state.canonicalize()?;
        for folder in &self.folders {
            let source = folder.host.canonicalize()?;
            if source != folder.host || !source.is_dir() {
                return Err(Error::InvalidHostFolder(
                    "host folder moved or is no longer a directory",
                ));
            }
            if source.starts_with(&state) || state.starts_with(&source) {
                return Err(Error::InvalidHostFolder(
                    "host folder must not include or be inside Shroom's state directory",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_names_are_non_root_and_safe_in_paths_and_commands() {
        for name in ["developer", "arctic", "user_2", "a-b", &"a".repeat(32)] {
            let user: GuestUser = name.parse().unwrap();
            assert_eq!(user.as_str(), name);
            assert_eq!(user.directory(), format!("/home/{name}/workspace"));
        }
        for name in [
            "",
            "root",
            "0",
            "-u",
            "Upper",
            "a b",
            "a/b",
            "a\nb",
            "a;id",
            "a$(id)",
            &"a".repeat(33),
        ] {
            assert!(
                matches!(name.parse::<GuestUser>(), Err(Error::InvalidGuestUser)),
                "{name:?}"
            );
        }
    }

    #[test]
    fn folders_reject_unsafe_destinations_overlap_and_runtime_storage_without_writes() {
        let root = tempfile::tempdir().unwrap();
        let host = root.path().join("host");
        let state = root.path().join("state");
        std::fs::create_dir(&host).unwrap();
        std::fs::create_dir(&state).unwrap();
        std::fs::write(host.join("sentinel"), "unchanged").unwrap();
        let folder =
            HostFolder::new(host.clone(), "/mnt/project".into(), FolderAccess::ReadWrite).unwrap();
        let options = WorkspaceOptions {
            folders: vec![folder.clone()],
            ..Default::default()
        };
        options.validate_sources(&state).unwrap();
        for guest in [
            "/",
            "/etc/ssh",
            "/home/developer",
            "/mnt",
            "/mnt/",
            "/mnt/../etc",
            "/mnt/./project",
            "/mnt//project",
            "/mnt/project/",
            "/mnt/a\nb",
            "mnt/project",
        ] {
            assert!(
                matches!(
                    HostFolder::new(host.clone(), guest.into(), FolderAccess::ReadOnly),
                    Err(Error::InvalidHostFolder(_))
                ),
                "{guest}"
            );
        }
        assert!(matches!(
            HostFolder::new(
                host.join("sentinel"),
                "/mnt/file".into(),
                FolderAccess::ReadOnly
            ),
            Err(Error::InvalidHostFolder(
                "host folder must be an existing directory"
            ))
        ));
        for guest in ["/mnt/project", "/mnt/project/child"] {
            let overlap = WorkspaceOptions {
                folders: vec![
                    folder.clone(),
                    HostFolder::new(host.clone(), guest.into(), FolderAccess::ReadOnly).unwrap(),
                ],
                ..Default::default()
            };
            assert!(matches!(
                overlap.validate(),
                Err(Error::InvalidHostFolder("guest folders must not overlap"))
            ));
        }
        for source in [state.clone(), root.path().to_owned()] {
            let dangerous = WorkspaceOptions {
                folders: vec![
                    HostFolder::new(source, "/mnt/state".into(), FolderAccess::ReadWrite).unwrap(),
                ],
                ..Default::default()
            };
            assert!(matches!(
                dangerous.validate_sources(&state),
                Err(Error::InvalidHostFolder(
                    "host folder must not include or be inside Shroom's state directory"
                ))
            ));
        }
        assert_eq!(
            std::fs::read_to_string(host.join("sentinel")).unwrap(),
            "unchanged"
        );
        assert_eq!(std::fs::read_dir(&state).unwrap().count(), 0);
        std::fs::rename(&host, root.path().join("moved")).unwrap();
        assert!(
            matches!(options.validate_sources(&state), Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound)
        );
    }
}
