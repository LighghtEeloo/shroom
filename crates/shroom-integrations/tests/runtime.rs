//! Opt-in verification of the public integration API against a real Shroom guest.

use std::{
    env, fs,
    path::PathBuf,
    process::{Command as HostCommand, Output, Stdio},
    time::Duration,
};

use shroom_core::{Config, Core, HostPublicKey, SshConnection, WORKSPACE_IMAGE};
use shroom_integrations::{Attachment, GuestWebUrl, RemoteCommand, Terminal};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::Command,
    time,
};

struct Runtime {
    root: PathBuf,
    state: PathBuf,
    config: Config,
    port: u16,
}

impl Runtime {
    async fn new() -> (Self, Core) {
        let root = tempfile::Builder::new()
            .prefix("sa-")
            .tempdir_in("/tmp")
            .unwrap()
            .keep();
        let state = root.join("s %");
        eprintln!("integration acceptance state: {}", state.display());
        let config = Config {
            runtime_executable: env::var_os("SHROOM_TEST_RUNTIME")
                .expect("set SHROOM_TEST_RUNTIME")
                .into(),
            firmware: env::var_os("SHROOM_TEST_FIRMWARE")
                .expect("set SHROOM_TEST_FIRMWARE")
                .into(),
        };
        let core = Core::open(state.clone(), config.clone()).await.unwrap();
        let mut import = HostCommand::new(&config.runtime_executable);
        import
            .env("MSB_HOME", state.join("microsandbox"))
            .env("MSB_CONFIG_PATH", state.join("microsandbox/config.json"))
            .args(["image", "load", "-i"])
            .arg(env::var_os("SHROOM_TEST_IMAGE_ARCHIVE").expect("set SHROOM_TEST_IMAGE_ARCHIVE"))
            .args(["--tag", WORKSPACE_IMAGE]);
        let output = Self::output(import).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let port = env::var("SHROOM_INTEGRATION_TEST_PORT")
            .unwrap_or_else(|_| "34522".into())
            .parse::<u16>()
            .unwrap();
        assert!((1024..=65532).contains(&port));
        (
            Self {
                root,
                state,
                config,
                port,
            },
            core,
        )
    }

    async fn output(command: HostCommand) -> Output {
        time::timeout(
            Duration::from_secs(30),
            Command::from(command)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap()
    }

    async fn run(attachment: &Attachment, program: &str, args: &[&str]) -> Output {
        let remote = args
            .iter()
            .try_fold(RemoteCommand::new(program).unwrap(), |remote, arg| {
                remote.with_arg(*arg)
            })
            .unwrap();
        Self::output(attachment.command(&remote, Terminal::None)).await
    }

    async fn get(port: u16) -> Vec<u8> {
        time::timeout(Duration::from_secs(10), async {
            let mut stream = loop {
                match TcpStream::connect(("127.0.0.1", port)).await {
                    Ok(stream) => break stream,
                    Err(_) => time::sleep(Duration::from_millis(20)).await,
                }
            };
            stream
                .write_all(b"GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
                .await
                .unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            response
        })
        .await
        .unwrap()
    }

    fn attachment(connection: SshConnection) -> Attachment {
        Attachment::new("shroom-integration-test".parse().unwrap(), connection).unwrap()
    }
}

#[tokio::test]
#[ignore = "requires the pinned runtime, prepared OCI archive, virtualization, and three free host ports"]
async fn ssh_export_commands_and_web_forwarding() {
    let (runtime, mut core) = Runtime::new().await;
    let name = "apps".parse().unwrap();
    let workspace = core
        .create(name, u32::from(runtime.port).try_into().unwrap())
        .await
        .unwrap();
    let name = workspace.name;
    let attachment = Runtime::attachment(workspace.ssh.unwrap());
    let identity = fs::read(&attachment.connection().identity_file).unwrap();
    let pin = fs::read(&attachment.connection().known_hosts_file).unwrap();
    let output = Runtime::run(&attachment, "id", &["-un"]).await;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"developer\n");
    let quoted = runtime.root.join("access % 'single' \"double\" \\slash");
    fs::create_dir(&quoted).unwrap();
    let mut copied = attachment.connection().clone();
    copied.identity_file = quoted.join("identity");
    copied.known_hosts_file = quoted.join("known_hosts");
    fs::copy(
        &attachment.connection().identity_file,
        &copied.identity_file,
    )
    .unwrap();
    fs::copy(
        &attachment.connection().known_hosts_file,
        &copied.known_hosts_file,
    )
    .unwrap();
    let copied = Runtime::attachment(copied);
    let output = Runtime::run(&copied, "id", &["-un"]).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"developer\n");
    let output = Runtime::run(&attachment, "pwd", &[]).await;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"/home/developer/workspace\n");

    let config = runtime.root.join("ssh_config");
    fs::write(&config, copied.ssh_config()).unwrap();
    let mut exported = HostCommand::new("ssh");
    exported
        .arg("-F")
        .arg(&config)
        .arg(attachment.alias().as_str())
        .arg("id -un");
    let output = Runtime::output(exported).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"developer\n");
    let literal = "spaces 'quotes' \"double\" ; $(touch unexpected) `touch unexpected` % $HOME";
    let output = Runtime::run(&attachment, "/usr/bin/printf", &["%s", literal]).await;
    assert!(output.status.success());
    assert_eq!(output.stdout, literal.as_bytes());
    assert!(
        Runtime::run(
            &attachment,
            "/bin/sh",
            &[
                "-c",
                "test ! -e unexpected && printf retained > integration-marker"
            ]
        )
        .await
        .status
        .success()
    );

    let other_identity = runtime.root.join("other-key");
    let mut keygen = HostCommand::new("ssh-keygen");
    keygen
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&other_identity);
    assert!(Runtime::output(keygen).await.status.success());
    let mut wrong = attachment.connection().clone();
    wrong.identity_file = other_identity.clone();
    let output = Runtime::run(&Runtime::attachment(wrong), "true", &[]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Permission denied"));
    let wrong_host: HostPublicKey = fs::read_to_string(other_identity.with_extension("pub"))
        .unwrap()
        .parse()
        .unwrap();
    let wrong_pin = runtime.root.join("wrong_known_hosts");
    fs::write(
        &wrong_pin,
        format!(
            "{} {}\n",
            wrong_host.alias(),
            wrong_host.to_openssh().unwrap()
        ),
    )
    .unwrap();
    let mut wrong = attachment.connection().clone();
    wrong.host_key_alias = wrong_host.alias();
    wrong.host_key = wrong_host;
    wrong.known_hosts_file = wrong_pin;
    let output = Runtime::run(&Runtime::attachment(wrong), "true", &[]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Host key verification failed"));
    assert_eq!(
        fs::read(&attachment.connection().identity_file).unwrap(),
        identity
    );
    assert_eq!(
        fs::read(&attachment.connection().known_hosts_file).unwrap(),
        pin
    );

    // Perl and its socket library are included through Git's Debian dependencies.
    // This service isolates transport acceptance from app installation and provider login.
    let service = r#"use IO::Socket::INET;
$| = 1;
my $server = IO::Socket::INET->new(LocalAddr => '127.0.0.1', LocalPort => 15497, Listen => 5, ReuseAddr => 1) or die $!;
open(my $pid, '>', '.integration-web.pid') or die $!; print $pid $$; close $pid;
print "http://127.0.0.1:15497/?token=fixture\n";
while (my $client = $server->accept()) {
    while (my $line = <$client>) { last if $line eq "\r\n"; }
    print $client "HTTP/1.0 200 OK\r\nContent-Length: 9\r\n\r\nworkspace";
    close $client;
}"#;
    let remote = RemoteCommand::new("perl")
        .unwrap()
        .with_arg("-e")
        .unwrap()
        .with_arg(service)
        .unwrap();
    let mut server = Command::from(attachment.command(&remote, Terminal::None))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(server.stdout.take().unwrap());
    let mut reported = String::new();
    time::timeout(Duration::from_secs(10), stdout.read_line(&mut reported))
        .await
        .unwrap()
        .unwrap();
    let guest: GuestWebUrl = reported.trim().parse().unwrap();
    let local_port = runtime.port + 1;
    let tunnel = attachment.web_tunnel(guest.clone(), u32::from(local_port).try_into().unwrap());
    assert_eq!(
        tunnel.local_url().as_str(),
        format!("http://127.0.0.1:{local_port}/?token=fixture")
    );
    let mut forward = Command::from(tunnel.command())
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let response = Runtime::get(local_port).await;
    assert!(response.ends_with(b"workspace"));
    assert!(forward.try_wait().unwrap().is_none());

    let conflict_port = runtime.port + 2;
    let occupied = TcpListener::bind(("127.0.0.1", conflict_port))
        .await
        .unwrap();
    let conflict = attachment.web_tunnel(guest, u32::from(conflict_port).try_into().unwrap());
    let output = Runtime::output(conflict.command()).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Could not request local forwarding"));
    assert!(
        Runtime::get(local_port).await.ends_with(b"workspace"),
        "a rejected forward preserves the existing service"
    );
    drop(occupied);
    forward.kill().await.unwrap();
    assert!(
        Runtime::run(
            &attachment,
            "/bin/sh",
            &[
                "-c",
                "kill \"$(cat .integration-web.pid)\" && rm .integration-web.pid"
            ]
        )
        .await
        .status
        .success()
    );
    time::timeout(Duration::from_secs(10), server.wait())
        .await
        .unwrap()
        .unwrap();

    core.stop(&name).await.unwrap();
    drop(core);
    let mut core = Core::open(runtime.state.clone(), runtime.config)
        .await
        .unwrap();
    let refreshed = Runtime::attachment(core.start(&name).await.unwrap().ssh.unwrap());
    assert_eq!(
        refreshed.connection().host_key,
        attachment.connection().host_key
    );
    assert_eq!(refreshed.alias(), attachment.alias());
    assert_eq!(
        Runtime::run(&refreshed, "cat", &["integration-marker"])
            .await
            .stdout,
        b"retained"
    );
    assert_eq!(
        fs::read(&refreshed.connection().identity_file).unwrap(),
        identity
    );
    assert_eq!(
        fs::read(&refreshed.connection().known_hosts_file).unwrap(),
        pin
    );
    core.stop(&name).await.unwrap();
    core.remove(&name).await.unwrap();
    drop(core);
    fs::remove_dir_all(runtime.root).unwrap();
}
