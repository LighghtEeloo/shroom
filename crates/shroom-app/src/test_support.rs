use shroom_core::{Config, HostPublicKey, SandboxStatus, SshConnection, Workspace};

use crate::model::{SessionInfo, WorkspaceView};

pub struct Fixture;

impl Fixture {
    pub fn session() -> Option<SessionInfo> {
        Some(SessionInfo {
            state_dir: "/tmp/shroom-preview".into(),
            config: Config {
                runtime_executable: "/opt/homebrew/opt/microsandbox/libexec/msb".into(),
                firmware: "/opt/homebrew/opt/microsandbox/libexec/libkrunfw.5.dylib".into(),
            },
        })
    }

    pub fn connection(name: &str) -> SshConnection {
        // Public key from RFC 8032 test vector 1. No private key or actual workspace is used.
        let host: HostPublicKey =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINdamAGCsQq31Uv+08lkBzoO4XLz2qYjJa8CGmj3B1Ea"
                .parse()
                .unwrap();
        let access = Self::session().unwrap().state_dir.join("access").join(name);
        SshConnection {
            endpoint: "127.0.0.1:2222".parse().unwrap(),
            user: "developer".into(),
            identity_file: access.join("client_ed25519"),
            known_hosts_file: access.join("known_hosts"),
            host_key_alias: host.alias(),
            host_key: host,
        }
    }

    pub fn workspace(name: &str, state: SandboxStatus) -> WorkspaceView {
        WorkspaceView {
            directory_editable: true,
            preference: Ok(Default::default()),
            workspace: Workspace {
                name: name.parse().unwrap(),
                state,
                options: Default::default(),
                ssh: (state == SandboxStatus::Running).then(|| Self::connection(name)),
            },
        }
    }
}
