use std::{fs, path::PathBuf, process::Command};

use shroom_core::{HostPublicKey, SshConnection};
use shroom_integrations::Attachment;

pub struct Fixture {
    pub root: tempfile::TempDir,
    pub connection: SshConnection,
}

impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let access = root
            .path()
            .join("access with % 'quotes' \"double\" \\slash");
        fs::create_dir(&access).unwrap();
        let identity = access.join("identity");
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&identity)
            .status()
            .unwrap();
        assert!(status.success());
        let host: HostPublicKey = fs::read_to_string(identity.with_extension("pub"))
            .unwrap()
            .parse()
            .unwrap();
        let known_hosts_file = access.join("known_hosts");
        fs::write(
            &known_hosts_file,
            format!("{} {}\n", host.alias(), host.to_openssh().unwrap()),
        )
        .unwrap();
        let directory = root
            .path()
            .join("project's \"files\"; $(touch SHOULD_NOT_EXIST)");
        fs::create_dir(&directory).unwrap();
        let connection = SshConnection {
            endpoint: "127.0.0.1:2222".parse().unwrap(),
            user: "developer".into(),
            identity_file: identity,
            host_key_alias: host.alias(),
            host_key: host,
            known_hosts_file,
            directory: directory.to_str().unwrap().into(),
        };
        Self { root, connection }
    }

    pub fn attachment(&self) -> Attachment {
        Attachment::new("shroom-test".parse().unwrap(), self.connection.clone()).unwrap()
    }

    pub fn config(&self) -> PathBuf {
        let path = self.root.path().join("ssh_config");
        fs::write(&path, self.attachment().ssh_config()).unwrap();
        path
    }
}
