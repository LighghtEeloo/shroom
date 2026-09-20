use std::{net::Ipv4Addr, process::Command, str::FromStr};

use derive_more::Display;
use shroom_core::HostPort;
use url::{Host, Url};

use crate::{Attachment, Error, Result};

/// An unprivileged web service port inside the guest.
#[derive(Clone, Copy, Debug, Display, Eq, PartialEq)]
pub struct GuestPort(u16);

impl GuestPort {
    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u32> for GuestPort {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        if !(1024..=65535).contains(&value) {
            return Err(Error::InvalidGuestPort(value));
        }
        Ok(Self(value as u16))
    }
}

/// The actual HTTP URL reported by a guest web app, including any authentication query/fragment.
/// Its contents may be sensitive; avoid logging it.
#[derive(Clone, Eq, PartialEq)]
pub struct GuestWebUrl(Url);

impl FromStr for GuestWebUrl {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(Error::InvalidWebUrl);
        }
        let url = Url::parse(value).map_err(|_| Error::InvalidWebUrl)?;
        let loopback = match url.host() {
            Some(Host::Domain("localhost")) => true,
            Some(Host::Ipv4(ip)) => ip == Ipv4Addr::LOCALHOST,
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if url.scheme() != "http"
            || !loopback
            || !url.username().is_empty()
            || url.password().is_some()
            || !url.port().is_some_and(|port| port >= 1024)
        {
            return Err(Error::InvalidWebUrl);
        }
        Ok(Self(url))
    }
}

impl GuestWebUrl {
    pub fn as_url(&self) -> &Url {
        &self.0
    }
}

/// A planned local forward. Its URL is usable only after SSH and the guest app are ready.
/// Starting or stopping the forward does not start or stop the guest application.
pub struct WebTunnel {
    attachment: Attachment,
    guest: GuestWebUrl,
    local_port: HostPort,
}

impl Attachment {
    pub fn web_tunnel(&self, guest: GuestWebUrl, local_port: HostPort) -> WebTunnel {
        WebTunnel {
            attachment: self.clone(),
            guest,
            local_port,
        }
    }
}

impl WebTunnel {
    /// Retain the guest's path, query, and fragment while changing only host and port.
    pub fn local_url(&self) -> Url {
        let mut url = self.guest.0.clone();
        url.set_host(Some("127.0.0.1")).expect("fixed host");
        url.set_port(Some(self.local_port.get())).expect("HTTP URL");
        url
    }

    /// Fail if OpenSSH cannot bind the requested host port. Application readiness is separate.
    pub fn command(&self) -> Command {
        let guest_host = match self.guest.0.host().expect("validated URL") {
            Host::Ipv6(ip) => format!("[{ip}]"),
            _ => "127.0.0.1".to_owned(),
        };
        let mut command = self.attachment.ssh_command();
        command
            .args([
                "-N",
                "-T",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "GatewayPorts=no",
            ])
            .arg("-L")
            .arg(format!(
                "127.0.0.1:{}:{guest_host}:{}",
                self.local_port,
                self.guest.0.port().expect("validated port")
            ))
            .arg(self.attachment.alias.as_str());
        command
    }
}
