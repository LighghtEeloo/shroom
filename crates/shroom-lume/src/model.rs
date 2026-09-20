use std::{fmt, net::IpAddr, path::PathBuf, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{Error, Result};

/// A local VM name that Lume will not normalize or interpret as a registry reference.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct VmName(String);

impl VmName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VmName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for VmName {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        if !(1..=48).contains(&value.len())
            || !value.bytes().next().is_some_and(alphanumeric)
            || !value.bytes().all(|byte| alphanumeric(byte) || byte == b'-')
        {
            return Err(Error::InvalidName);
        }
        Ok(Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for VmName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum GuestOs {
    #[serde(rename = "linux")]
    Linux,
    #[serde(rename = "macOS", alias = "macos", alias = "darwin")]
    MacOs,
}

/// Observed native state. Unknown future states never become `Stopped`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmState {
    Stopped,
    Running,
    Pending,
    Provisioning,
    StaleProvisioning,
    Pulling,
    Unknown(String),
}

impl<'de> Deserialize<'de> for VmState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "stopped" => Self::Stopped,
            "running" => Self::Running,
            "pending" => Self::Pending,
            "provisioning" => Self::Provisioning,
            "provisioning (stale)" => Self::StaleProvisioning,
            "pulling" => Self::Pulling,
            other => Self::Unknown(other.to_owned()),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct DiskSize {
    pub allocated: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SharedDirectory {
    pub host_path: PathBuf,
    pub tag: String,
    pub read_only: bool,
}

/// Native inspection data. Pulling responses contain only name, state, and progress.
/// Optional fields reflect upstream absence and do not imply zero resources or readiness.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Vm {
    pub name: VmName,
    #[serde(rename = "status")]
    pub state: VmState,
    pub os: Option<GuestOs>,
    pub cpu_count: Option<u32>,
    pub memory_size: Option<u64>,
    pub disk_size: Option<DiskSize>,
    pub ip_address: Option<IpAddr>,
    /// Native reachability hint; this is not authenticated SSH readiness.
    pub ssh_available: Option<bool>,
    pub location_name: Option<String>,
    pub shared_directories: Option<Vec<SharedDirectory>>,
    pub provisioning_operation: Option<String>,
    pub download_progress: Option<f64>,
}

/// Acknowledgement of an asynchronous create or start, not its completion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Accepted {
    pub name: VmName,
    #[serde(rename = "status")]
    pub state: VmState,
}

#[derive(Clone, Debug)]
pub enum Guest {
    /// Allocate an empty Linux VM. Clone a prepared VM to obtain a bootable guest.
    Linux,
    /// Install from a local IPSW. Account and SSH provisioning remain separate.
    MacOs { restore_image: PathBuf },
}

#[derive(Clone, Debug)]
pub struct CreateVm {
    pub name: VmName,
    pub guest: Guest,
    pub cpus: u16,
    pub memory_mib: u32,
    pub disk_gib: u32,
}

impl CreateVm {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.cpus == 0 || self.memory_mib == 0 || self.disk_gib == 0 {
            return Err(Error::InvalidCreate(
                "CPU, memory, and disk allocations must be positive",
            ));
        }
        if let Guest::MacOs { restore_image } = &self.guest
            && (!restore_image.is_absolute()
                || restore_image
                    .to_str()
                    .is_none_or(|path| path.chars().any(char::is_control)))
        {
            return Err(Error::InvalidCreate(
                "IPSW must have an absolute UTF-8 path without control characters",
            ));
        }
        Ok(())
    }
}
