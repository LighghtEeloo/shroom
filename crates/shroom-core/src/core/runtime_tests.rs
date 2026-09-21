//! Opt-in acceptance against the installed runtime, with no mock hypervisor.

use super::*;
use std::{env, process::Stdio};
use tokio::{io::AsyncWriteExt, process::Command};

struct Fixture {
    directory: PathBuf,
    state: PathBuf,
    config: Config,
    port: u16,
}

impl Fixture {
    fn config() -> Config {
        Config {
            runtime_executable: env::var_os("SHROOM_TEST_RUNTIME")
                .expect("set SHROOM_TEST_RUNTIME")
                .into(),
            firmware: env::var_os("SHROOM_TEST_FIRMWARE")
                .expect("set SHROOM_TEST_FIRMWARE")
                .into(),
        }
    }

    async fn new() -> (Self, Core) {
        // Keep artifacts on a failed assertion, including any detached runtime's disks.
        let directory = tempfile::Builder::new()
            .prefix("sh-")
            .tempdir_in("/tmp")
            .unwrap()
            .keep();
        let state = directory.join("s %");
        eprintln!("acceptance state: {}", state.display());
        let config = Self::config();
        let core = Core::open(state.clone(), config.clone()).await.unwrap();
        let archive = PathBuf::from(
            env::var_os("SHROOM_TEST_IMAGE_ARCHIVE").expect("set SHROOM_TEST_IMAGE_ARCHIVE"),
        );
        core.scoped(microsandbox::Image::load(
            &archive,
            vec![WORKSPACE_IMAGE.into()],
        ))
        .await
        .unwrap();
        let port = env::var("SHROOM_TEST_PORT")
            .unwrap_or_else(|_| "34222".into())
            .parse::<u16>()
            .unwrap();
        assert!((1024..=65495).contains(&port));
        (
            Self {
                directory,
                state,
                config,
                port,
            },
            core,
        )
    }

    async fn ssh(connection: &crate::SshConnection, command: &str) -> std::process::Output {
        Helper::output(connection.command().arg(command), Duration::from_secs(15))
            .await
            .unwrap()
    }

    async fn admin(core: &Core, name: &WorkspaceName, command: &str) {
        core.scoped(async {
            let sandbox = Sandbox::get(name.as_str())
                .await
                .unwrap()
                .connect()
                .await
                .unwrap();
            let result = sandbox
                .exec_with("/bin/sh", |e| {
                    e.args(["-c", command])
                        .user("root")
                        .timeout(Duration::from_secs(10))
                })
                .await
                .unwrap();
            assert!(result.status().success, "{:?}", result.stderr());
        })
        .await;
    }

    async fn assert_sudo(connection: &crate::SshConnection) {
        let output = Self::ssh(
            connection,
            "id -u; sudo -k -n id -u && sudo -k -n /usr/sbin/visudo -c >/dev/null && stat -c '%u:%g:%a' /etc/sudoers.d/shroom-workspace",
        )
        .await;
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(output.stdout, b"1000\n0\n0:0:440\n");
        let denied = Self::ssh(
            connection,
            "printf forbidden >> /etc/sudoers.d/shroom-workspace",
        )
        .await;
        assert!(!denied.status.success());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("Permission denied"));
        assert_eq!(
            Self::ssh(connection, "sudo -k -n cat /etc/sudoers.d/shroom-workspace")
                .await
                .stdout,
            format!("{} ALL=(ALL:ALL) NOPASSWD: ALL\n", connection.user).as_bytes()
        );
    }

    async fn sftp(connection: &crate::SshConnection, batch: String) -> std::process::Output {
        let mut child = Command::new("sftp")
            .args([
                "-F",
                "/dev/null",
                "-b",
                "-",
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
                "ConnectTimeout=3",
            ])
            .arg("-o")
            .arg(format!("HostKeyAlias={}", connection.host_key_alias))
            .arg("-o")
            .arg(format!(
                "UserKnownHostsFile=\"{}\"",
                connection
                    .known_hosts_file
                    .to_str()
                    .unwrap()
                    .replace('%', "%%")
            ))
            .arg("-o")
            .arg(format!(
                "IdentityFile=\"{}\"",
                connection
                    .identity_file
                    .to_str()
                    .unwrap()
                    .replace('%', "%%")
            ))
            .arg("-P")
            .arg(connection.endpoint.port().to_string())
            .arg(format!("{}@{}", connection.user, connection.endpoint.ip()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(batch.as_bytes())
            .await
            .unwrap();
        time::timeout(Duration::from_secs(15), child.wait_with_output())
            .await
            .unwrap()
            .unwrap()
    }

    fn leaf(error: &Error) -> &Error {
        match error {
            Error::Operation { source, .. } => Self::leaf(source),
            error => error,
        }
    }
}

#[tokio::test]
#[ignore = "invoked by runtime_contract in a separate process"]
async fn caller_process() {
    let Some(state) = env::var_os("SHROOM_TEST_CHILD_STATE") else {
        return;
    };
    let port = env::var("SHROOM_TEST_CHILD_PORT")
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let mut core = Core::open(state.into(), Fixture::config()).await.unwrap();
    assert!(
        core.create(
            "alpha".parse().unwrap(),
            port.try_into().unwrap(),
            Default::default()
        )
        .await
        .unwrap()
        .ssh
        .is_some()
    );
}

#[tokio::test]
#[ignore = "requires the pinned runtime pair, prepared OCI archive, virtualization, and a free port range"]
async fn runtime_contract() {
    let (fixture, mut core) = Fixture::new().await;
    let alpha: WorkspaceName = "alpha".parse().unwrap();
    let beta: WorkspaceName = "beta".parse().unwrap();
    let partial: WorkspaceName = "partial".parse().unwrap();
    assert!(matches!(
        Core::open(fixture.state.clone(), fixture.config.clone()).await,
        Err(Error::CoreInUse(_))
    ));
    assert!(core.list().await.unwrap().is_empty());
    assert!(matches!(
        Fixture::leaf(&core.start(&alpha).await.unwrap_err()),
        Error::Sdk(MicrosandboxError::SandboxNotFound(_))
    ));
    assert!(!core.access(&alpha).directory.exists());

    drop(core);
    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "core::runtime_tests::caller_process",
            "--nocapture",
        ])
        .env("SHROOM_TEST_CHILD_STATE", &fixture.state)
        .env("SHROOM_TEST_CHILD_PORT", fixture.port.to_string())
        .kill_on_drop(true)
        .status()
        .await
        .unwrap();
    assert!(child.success());
    eprintln!("caller exited; checking catalog and SSH identities");
    core = Core::open(fixture.state.clone(), fixture.config.clone())
        .await
        .unwrap();
    let a = core.get(&alpha).await.unwrap().ssh.unwrap();
    a.probe().await.unwrap();
    assert_eq!(Fixture::ssh(&a, "id -un").await.stdout, b"developer\n");
    Fixture::assert_sudo(&a).await;
    let original_key = fs::read(&a.identity_file).unwrap();
    let original_pin = fs::read(&a.known_hosts_file).unwrap();
    assert!(matches!(
        Fixture::leaf(
            &core
                .create(
                    alpha.clone(),
                    u32::from(fixture.port + 1).try_into().unwrap(),
                    Default::default()
                )
                .await
                .unwrap_err()
        ),
        Error::Sdk(MicrosandboxError::SandboxAlreadyExists(_))
    ));
    assert!(matches!(
        Fixture::leaf(
            &core
                .create(
                    beta.clone(),
                    u32::from(fixture.port).try_into().unwrap(),
                    Default::default()
                )
                .await
                .unwrap_err()
        ),
        Error::PortInUse { .. }
    ));
    assert!(!core.access(&beta).directory.exists());
    assert_eq!(fs::read(&a.identity_file).unwrap(), original_key);
    assert_eq!(fs::read(&a.known_hosts_file).unwrap(), original_pin);

    let occupied_name: WorkspaceName = "occupied".parse().unwrap();
    let occupied = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, fixture.port + 3)).unwrap();
    let error = core
        .create(
            occupied_name.clone(),
            u32::from(fixture.port + 3).try_into().unwrap(),
            Default::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error,
        Error::Operation { stage: Stage::CreateSandbox, source, .. } if matches!(source.as_ref(), Error::Sdk(_)))
            || matches!(&error, Error::Operation { stage: Stage::Probe, source, .. } if matches!(source.as_ref(), Error::SshUnavailable(_))),
        "{error:?}"
    );
    assert!(core.access(&occupied_name).directory.exists());
    drop(occupied);
    // This runtime may retain a running VM after a publisher bind failure. Cleanup stays explicit.
    if core
        .get(&occupied_name)
        .await
        .is_ok_and(|workspace| workspace.state == SandboxStatus::Running)
    {
        assert!(matches!(
            Fixture::leaf(&core.remove(&occupied_name).await.unwrap_err()),
            Error::Sdk(MicrosandboxError::SandboxStillRunning(_))
        ));
        core.stop(&occupied_name).await.unwrap();
    }
    core.remove(&occupied_name).await.unwrap();

    let b = core
        .create(
            beta.clone(),
            u32::from(fixture.port + 1).try_into().unwrap(),
            Default::default(),
        )
        .await
        .unwrap()
        .ssh
        .unwrap();
    eprintln!("two workspaces ready; checking trust rejection");
    assert_ne!(a.host_key, b.host_key);
    assert_eq!(core.list().await.unwrap().len(), 2);
    assert!(matches!(
        Fixture::leaf(&core.remove(&alpha).await.unwrap_err()),
        Error::Sdk(MicrosandboxError::SandboxStillRunning(_))
    ));
    assert_eq!(fs::read(&a.known_hosts_file).unwrap(), original_pin);

    let wrong_client = crate::SshConnection {
        identity_file: a.identity_file.clone(),
        ..b.clone()
    };
    let output = Fixture::ssh(&wrong_client, "true").await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Permission denied"));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Permission denied (publickey)"),
        "password authentication must not be offered"
    );
    let root_client = crate::SshConnection {
        user: "root".into(),
        ..a.clone()
    };
    assert!(!Fixture::ssh(&root_client, "true").await.status.success());
    let stale = crate::SshConnection {
        endpoint: b.endpoint,
        ..a.clone()
    };
    assert!(matches!(stale.probe().await, Err(Error::HostKeyMismatch)));
    assert_eq!(fs::read(&a.known_hosts_file).unwrap(), original_pin);
    fs::write(&a.known_hosts_file, fs::read(&b.known_hosts_file).unwrap()).unwrap();
    assert!(matches!(
        Fixture::leaf(&core.start(&alpha).await.unwrap_err()),
        Error::HostKeyMismatch
    ));
    assert_eq!(
        fs::read(&a.known_hosts_file).unwrap(),
        fs::read(&b.known_hosts_file).unwrap()
    );
    fs::write(&a.known_hosts_file, &original_pin).unwrap();
    eprintln!("trust checks passed; checking endpoint refresh and guest permissions");

    // A real OpenSSH forwarding client provides a second endpoint to the same host identity.
    let args = a.command();
    let mut tunnel = Command::new("ssh")
        .args([
            "-N",
            "-o",
            "ClearAllForwardings=no",
            "-o",
            "ExitOnForwardFailure=yes",
            "-L",
        ])
        .arg(format!("127.0.0.1:{}:127.0.0.1:22", fixture.port + 2))
        .args(args.as_std().get_args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let changed = crate::SshConnection {
        endpoint: SocketAddr::from((Ipv4Addr::LOCALHOST, fixture.port + 2)),
        ..a.clone()
    };
    changed.probe().await.unwrap();
    tunnel.kill().await.unwrap();
    tunnel.wait().await.unwrap();
    assert_eq!(fs::read(&a.known_hosts_file).unwrap(), original_pin);

    Fixture::admin(&core, &alpha, "printf guard > /root/shroom-guard; chmod 0600 /root/shroom-guard; chmod 000 /usr/local/sbin/shroom-ssh").await;
    // Running start must not relaunch the now non-executable helper.
    core.start(&alpha).await.unwrap();
    Fixture::admin(&core, &alpha, "chmod 0755 /usr/local/sbin/shroom-ssh").await;
    assert!(Fixture::ssh(&a, "printf retained > /home/developer/workspace/retained; printf '#!/bin/sh\\necho installed\\n' > /home/developer/workspace/tool; chmod +x /home/developer/workspace/tool").await.status.success());
    assert!(
        !Fixture::ssh(&a, "printf forbidden > /root/shroom-guard")
            .await
            .status
            .success()
    );
    assert!(
        Fixture::ssh(
            &a,
            "test -z \"${SHROOM_TEST_RUNTIME+x}\" && test ! -e /home/developer/.ssh/id_ed25519"
        )
        .await
        .status
        .success()
    );
    let upload = fixture.directory.join("upload");
    fs::write(&upload, "sftp-data").unwrap();
    assert!(
        Fixture::sftp(
            &a,
            format!(
                "put {} /home/developer/workspace/upload\n",
                upload.display()
            )
        )
        .await
        .status
        .success()
    );
    assert!(
        !Fixture::sftp(&a, format!("put {} /root/shroom-guard\n", upload.display()))
            .await
            .status
            .success()
    );
    assert_eq!(
        Fixture::ssh(&a, "cat /home/developer/workspace/upload")
            .await
            .stdout,
        b"sftp-data"
    );
    assert!(
        !Fixture::ssh(&b, "test -e /home/developer/workspace/retained")
            .await
            .status
            .success()
    );
    assert!(
        Fixture::ssh(
            &a,
            "curl -fsS --connect-timeout 5 --max-time 10 https://example.com >/dev/null"
        )
        .await
        .status
        .success()
    );
    assert!(
        !Fixture::ssh(
            &a,
            "curl -fsS --connect-timeout 2 --max-time 3 http://192.168.1.1 >/dev/null"
        )
        .await
        .status
        .success()
    );

    let route = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    route.connect((Ipv4Addr::new(192, 0, 2, 1), 80)).unwrap();
    let host_ip = route.local_addr().unwrap().ip();
    assert!(!host_ip.is_loopback());
    assert!(
        std::net::TcpStream::connect_timeout(
            &SocketAddr::new(host_ip, fixture.port),
            Duration::from_millis(500)
        )
        .is_err()
    );
    let host_service = std::net::TcpListener::bind((host_ip, fixture.port + 4)).unwrap();
    assert!(
        !Fixture::ssh(
            &a,
            &format!(
                "timeout 3 bash -c 'exec 3<>/dev/tcp/{host_ip}/{}'",
                fixture.port + 4
            )
        )
        .await
        .status
        .success()
    );
    drop(host_service);

    core.stop(&alpha).await.unwrap();
    eprintln!("guest checks passed; checking persistence and incomplete access");
    core.stop(&alpha).await.unwrap();
    assert!(core.get(&alpha).await.unwrap().ssh.is_none());
    assert!(matches!(
        Fixture::leaf(
            &core
                .create(
                    partial.clone(),
                    u32::from(fixture.port).try_into().unwrap(),
                    Default::default()
                )
                .await
                .unwrap_err()
        ),
        Error::PortInUse { .. }
    ));
    fs::remove_file(fixture.state.join("access/alpha/client_ed25519.pub")).unwrap();
    assert!(matches!(
        Fixture::leaf(&core.start(&alpha).await.unwrap_err()),
        Error::AccessIncomplete(_)
    ));
    assert_eq!(
        core.get(&alpha).await.unwrap().state,
        SandboxStatus::Stopped
    );
    let public = ssh_key::PrivateKey::from_openssh(&original_key)
        .unwrap()
        .public_key()
        .to_openssh()
        .unwrap();
    fs::write(
        fixture.state.join("access/alpha/client_ed25519.pub"),
        public,
    )
    .unwrap();
    let restarted = core.start(&alpha).await.unwrap().ssh.unwrap();
    Fixture::assert_sudo(&restarted).await;
    assert_eq!(restarted.host_key, a.host_key);
    assert_eq!(fs::read(&restarted.identity_file).unwrap(), original_key);
    assert_eq!(
        Fixture::ssh(
            &restarted,
            "cat /home/developer/workspace/retained; /home/developer/workspace/tool"
        )
        .await
        .stdout,
        b"retainedinstalled\n"
    );

    // SDK zero-budget shutdown must preserve the runtime, its disks, and access material.
    core.scoped(async {
        let handle = Sandbox::get(alpha.as_str()).await.unwrap();
        assert!(matches!(
            handle.stop_with_timeout(Duration::ZERO).await,
            Err(MicrosandboxError::StopTimeout { .. })
        ));
    })
    .await;
    assert_eq!(
        core.get(&alpha).await.unwrap().state,
        SandboxStatus::Running
    );
    assert_eq!(fs::read(&a.identity_file).unwrap(), original_key);

    core.scoped(async {
        Sandbox::get(alpha.as_str())
            .await
            .unwrap()
            .pause()
            .await
            .unwrap()
    })
    .await;
    assert!(matches!(
        Fixture::leaf(&core.remove(&alpha).await.unwrap_err()),
        Error::Sdk(MicrosandboxError::SandboxStillRunning(_))
    ));
    assert!(matches!(
        Fixture::leaf(&core.start(&alpha).await.unwrap_err()),
        Error::Sdk(MicrosandboxError::SandboxNotRunning(_))
    ));
    assert_eq!(fs::read(&a.identity_file).unwrap(), original_key);
    core.scoped(async {
        Sandbox::get(alpha.as_str())
            .await
            .unwrap()
            .resume()
            .await
            .unwrap()
    })
    .await;

    // Go beyond the SDK's default twenty-record page, keeping only two VMs resident.
    let mut pages = Vec::new();
    for index in 0..19_u16 {
        let name: WorkspaceName = format!("page-{index}").parse().unwrap();
        core.create(
            name.clone(),
            u32::from(fixture.port + 10 + index).try_into().unwrap(),
            Default::default(),
        )
        .await
        .unwrap();
        core.stop(&name).await.unwrap();
        pages.push(name);
    }
    assert_eq!(core.list().await.unwrap().len(), 21);
    fs::remove_file(core.access(&pages[0]).directory.join("known_hosts")).unwrap();
    assert!(
        core.list()
            .await
            .unwrap()
            .iter()
            .any(|workspace| workspace.name == pages[0] && workspace.ssh.is_none())
    );
    assert!(matches!(
        Fixture::leaf(&core.start(&pages[0]).await.unwrap_err()),
        Error::AccessIncomplete(_)
    ));
    for name in pages {
        core.remove(&name).await.unwrap();
    }

    core.access(&partial).create().await.unwrap();
    eprintln!("persistence checks passed; checking partial cleanup");
    assert!(matches!(
        Fixture::leaf(
            &core
                .create(
                    partial.clone(),
                    u32::from(fixture.port + 2).try_into().unwrap(),
                    Default::default()
                )
                .await
                .unwrap_err()
        ),
        Error::AccessConflict(_)
    ));
    core.remove(&partial).await.unwrap();
    core.remove(&partial).await.unwrap();
    core.stop(&alpha).await.unwrap();
    core.stop(&beta).await.unwrap();
    core.remove(&alpha).await.unwrap();
    core.remove(&beta).await.unwrap();
    core.remove(&alpha).await.unwrap();
    assert!(core.list().await.unwrap().is_empty());
    assert!(!fixture.state.join("access/alpha").exists());
    drop(core);
    fs::remove_dir_all(&fixture.directory).unwrap();
}

#[tokio::test]
#[ignore = "requires the pinned runtime pair, prepared OCI archive, virtualization, and a free port range"]
async fn custom_account_and_shared_folders_persist_without_modifying_readonly_host_files() {
    use crate::{FolderAccess, HostFolder, WorkspaceOptions};

    let (fixture, mut core) = Fixture::new().await;
    let host = fixture.directory.join("shared folder");
    let readonly = fixture.directory.join("read only");
    fs::create_dir(&host).unwrap();
    fs::create_dir(&readonly).unwrap();
    fs::write(host.join("from-host"), "host-data").unwrap();
    fs::write(readonly.join("sentinel"), "unchanged").unwrap();
    let options = WorkspaceOptions {
        user: "arctic".parse().unwrap(),
        folders: vec![
            HostFolder::new(host.clone(), "/mnt/project".into(), FolderAccess::ReadWrite).unwrap(),
            HostFolder::new(
                readonly.clone(),
                "/mnt/reference".into(),
                FolderAccess::ReadOnly,
            )
            .unwrap(),
        ],
    };
    let name: WorkspaceName = "shared".parse().unwrap();
    let invalid = WorkspaceOptions {
        folders: vec![options.folders[0].clone(), options.folders[0].clone()],
        ..options.clone()
    };
    let error = core
        .create(
            name.clone(),
            u32::from(fixture.port).try_into().unwrap(),
            invalid,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        Fixture::leaf(&error),
        Error::InvalidHostFolder("guest folders must not overlap")
    ));
    assert!(core.list().await.unwrap().is_empty());
    assert!(!core.access(&name).directory.exists());
    let created = core
        .create(
            name.clone(),
            u32::from(fixture.port).try_into().unwrap(),
            options.clone(),
        )
        .await
        .unwrap();
    assert_eq!(created.options, options);
    let ssh = created.ssh.unwrap();
    assert_eq!(ssh.user, "arctic");
    Fixture::assert_sudo(&ssh).await;
    assert_eq!(
        Fixture::ssh(
            &ssh,
            "id -un; id -u; printf '%s\\n' \"$HOME\"; cat /mnt/project/from-host"
        )
        .await
        .stdout,
        b"arctic\n1000\n/home/arctic\nhost-data"
    );
    assert!(
        Fixture::ssh(&ssh, "printf guest-data > /mnt/project/from-guest")
            .await
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(host.join("from-guest")).unwrap(),
        "guest-data"
    );
    assert_eq!(
        Fixture::ssh(&ssh, "cat /mnt/reference/sentinel")
            .await
            .stdout,
        b"unchanged"
    );
    for command in [
        "printf changed > /mnt/reference/sentinel",
        "sudo -k -n sh -c 'printf changed > /mnt/reference/sentinel'",
    ] {
        let rejected = Fixture::ssh(&ssh, command).await;
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .to_lowercase()
                .contains("read-only"),
            "{:?}",
            rejected.stderr
        );
    }
    assert_eq!(
        fs::read_to_string(readonly.join("sentinel")).unwrap(),
        "unchanged"
    );
    let mut old_user = ssh.clone();
    old_user.user = "developer".into();
    assert!(!Fixture::ssh(&old_user, "true").await.status.success());
    core.stop(&name).await.unwrap();
    drop(core);
    let mut core = Core::open(fixture.state.clone(), fixture.config.clone())
        .await
        .unwrap();
    assert_eq!(core.get(&name).await.unwrap().options, options);
    let moved = fixture.directory.join("temporarily moved");
    fs::rename(&host, &moved).unwrap();
    assert_eq!(core.get(&name).await.unwrap().options, options);
    assert!(
        matches!(Fixture::leaf(&core.start(&name).await.unwrap_err()), Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound)
    );
    assert_eq!(core.get(&name).await.unwrap().state, SandboxStatus::Stopped);
    fs::rename(&moved, &host).unwrap();
    fs::write(host.join("from-host"), "after-restart").unwrap();
    let restarted = core.start(&name).await.unwrap().ssh.unwrap();
    Fixture::assert_sudo(&restarted).await;
    assert_eq!(restarted.user, "arctic");
    assert_eq!(restarted.host_key, ssh.host_key);
    assert_eq!(
        Fixture::ssh(
            &restarted,
            "cat /mnt/project/from-host /mnt/project/from-guest"
        )
        .await
        .stdout,
        b"after-restartguest-data"
    );
    assert!(
        !Fixture::ssh(&restarted, "rm /mnt/reference/sentinel")
            .await
            .status
            .success()
    );
    core.stop(&name).await.unwrap();
    core.remove(&name).await.unwrap();
    assert_eq!(
        fs::read_to_string(host.join("from-guest")).unwrap(),
        "guest-data"
    );
    assert_eq!(
        fs::read_to_string(readonly.join("sentinel")).unwrap(),
        "unchanged"
    );
    let collision: WorkspaceName = "collision".parse().unwrap();
    let error = core
        .create(
            collision.clone(),
            u32::from(fixture.port).try_into().unwrap(),
            WorkspaceOptions {
                user: "daemon".parse().unwrap(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(Fixture::leaf(&error), Error::GuestHelperFailed { code: 2, diagnostic } if diagnostic.contains("guest username already exists"))
    );
    Fixture::admin(&core, &collision, "test \"$(id -u developer)\" = 1000 && test ! -e /etc/shroom/authorized_keys && test ! -e /etc/ssh/ssh_host_ed25519_key && test ! -e /etc/sudoers.d/shroom-workspace").await;
    // The collision leaves an unprovisioned guest for exercising outdated-image rejection.
    for binary in ["/usr/bin/sudo", "/usr/sbin/visudo"] {
        Fixture::admin(&core, &collision, &format!("mv {binary} {binary}.disabled")).await;
        core.scoped(async {
            let sandbox = Sandbox::get(collision.as_str())
                .await
                .unwrap()
                .connect()
                .await
                .unwrap();
            let output = sandbox
                .exec_with("/bin/sh", |exec| {
                    exec.args([GUEST_HELPER, "provision", "arctic"])
                        .user("root")
                        .cwd("/")
                        .stdin_null()
                        .timeout(Duration::from_secs(10))
                })
                .await
                .unwrap();
            assert_eq!(output.status().code, 2);
            assert_eq!(
                output.stderr().unwrap().trim(),
                "workspace image is missing sudo; rebuild and reimport it"
            );
        })
        .await;
        Fixture::admin(&core, &collision, &format!("mv {binary}.disabled {binary}; test \"$(id -u developer)\" = 1000 && ! getent passwd arctic && test ! -e /etc/shroom/authorized_keys && test ! -e /etc/ssh/ssh_host_ed25519_key && test ! -e /etc/sudoers.d/shroom-workspace")).await;
    }
    core.stop(&collision).await.unwrap();
    core.remove(&collision).await.unwrap();
    drop(core);
    fs::remove_dir_all(fixture.directory).unwrap();
}
