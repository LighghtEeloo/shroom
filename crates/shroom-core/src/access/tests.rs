use super::*;

struct Fixture {
    _root: tempfile::TempDir,
    access: Access,
    host: HostPublicKey,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let access = Access {
            directory: root.path().join("access"),
        };
        let client = access.create().await.unwrap();
        let host = client.to_openssh().unwrap().parse().unwrap();
        Self {
            _root: root,
            access,
            host,
        }
    }
}

#[tokio::test]
async fn creation_permissions_and_immutable_pins() {
    let fixture = Fixture::new().await;
    let access = &fixture.access;
    assert!(access.read().unwrap().is_none());
    access.pin(&fixture.host).unwrap();
    assert_eq!(access.read().unwrap().unwrap().host_key, fixture.host);
    assert_eq!(
        fs::metadata(&access.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let private = access.directory.join("client_ed25519");
    assert_eq!(
        fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let original = fs::read(&private).unwrap();
    let pin = fs::read(access.directory.join("known_hosts")).unwrap();
    assert!(matches!(
        access.create().await,
        Err(Error::AccessConflict(_))
    ));
    assert!(
        matches!(access.pin(&fixture.host), Err(Error::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists)
    );
    assert_eq!(fs::read(&private).unwrap(), original);
    assert_eq!(fs::read(access.directory.join("known_hosts")).unwrap(), pin);
}

#[tokio::test]
async fn incomplete_and_malformed_access_are_distinct() {
    let fixture = Fixture::new().await;
    let access = &fixture.access;
    assert!(access.read().unwrap().is_none());
    let public_path = access.directory.join("client_ed25519.pub");
    let public = fs::read(&public_path).unwrap();
    fs::write(&public_path, "not an SSH public key").unwrap();
    assert!(matches!(access.read(), Err(Error::Key(_))));
    fs::write(&public_path, public).unwrap();
    access.pin(&fixture.host).unwrap();
    fs::remove_file(&public_path).unwrap();
    assert!(access.read().unwrap().is_none());
    fs::write(access.directory.join("known_hosts"), "broken pin").unwrap();
    assert!(
        access.read().is_err(),
        "an absent client file must not hide a malformed pin"
    );
    access.remove().unwrap();
    access.remove().unwrap();
    assert!(access.read().unwrap().is_none());
}

#[tokio::test]
async fn mismatched_client_halves_and_unsafe_files_are_rejected() {
    let a = Fixture::new().await;
    let b = Fixture::new().await;
    a.access.pin(&a.host).unwrap();
    let private_path = a.access.directory.join("client_ed25519");
    let original = fs::read(&private_path).unwrap();
    let public_path = a.access.directory.join("client_ed25519.pub");
    fs::write(&public_path, b.host.to_openssh().unwrap()).unwrap();
    assert!(matches!(
        a.access.read(),
        Err(Error::InvalidAccess(
            "client public and private keys disagree"
        ))
    ));
    assert_eq!(fs::read(&private_path).unwrap(), original);
    fs::write(&public_path, a.host.to_openssh().unwrap()).unwrap();
    fs::set_permissions(&private_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        a.access.read(),
        Err(Error::InvalidAccess(
            "client private key must have mode 0600"
        ))
    ));
    fs::set_permissions(&private_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&public_path).unwrap();
    std::os::unix::fs::symlink(b.access.directory.join("client_ed25519.pub"), &public_path)
        .unwrap();
    assert!(matches!(
        a.access.read(),
        Err(Error::InvalidAccess("expected a small regular access file"))
    ));
}

#[tokio::test]
async fn trust_aliases_bind_only_to_one_validated_ed25519_key() {
    let a = Fixture::new().await;
    let b = Fixture::new().await;
    let text = a.host.to_openssh().unwrap();
    let alias = a.host.alias();
    assert!(
        alias
            .as_str()
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    );
    assert_ne!(alias, b.host.alias());
    let with_comment: HostPublicKey = format!("{text} a comment\n").parse().unwrap();
    assert_eq!(a.host.alias(), with_comment.alias());
    let valid = format!("{alias} {text}\n");
    assert_eq!(Access::parse_pin(&valid).unwrap(), a.host);
    for invalid in [
        format!("other {text}"),
        format!("{valid}{valid}"),
        format!("{alias} ssh-ed25519 invalid"),
    ] {
        assert!(Access::parse_pin(&invalid).is_err());
    }
    let rsa = Helper::output(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "rsa", "-b", "1024", "-N", "", "-f"])
            .arg(a.access.directory.join("rsa")),
        HELPER_TIMEOUT,
    )
    .await
    .unwrap();
    assert!(rsa.status.success());
    let rsa = fs::read_to_string(a.access.directory.join("rsa.pub")).unwrap();
    assert!(matches!(
        rsa.parse::<HostPublicKey>(),
        Err(Error::InvalidAccess("expected an Ed25519 key"))
    ));
    assert!(matches!(
        rsa.parse::<ClientPublicKey>(),
        Err(Error::InvalidAccess("expected an Ed25519 key"))
    ));
}

#[tokio::test]
async fn ssh_options_ignore_ambient_configuration_and_agents() {
    let fixture = Fixture::new().await;
    let connection = fixture.access.connection(
        Material {
            host_key: fixture.host,
        },
        "127.0.0.1:2222".parse().unwrap(),
        &GuestUser::default(),
    );
    let command = connection.command();
    let output = Helper::output(
        Command::new("ssh")
            .arg("-G")
            .args(command.as_std().get_args()),
        HELPER_TIMEOUT,
    )
    .await
    .unwrap();
    assert!(output.status.success());
    let config = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "batchmode yes",
        "identitiesonly yes",
        "identityagent none",
        "stricthostkeychecking true",
        "checkhostip no",
        "globalknownhostsfile /dev/null",
        "hostkeyalgorithms ssh-ed25519",
    ] {
        assert!(
            config.lines().any(|line| line == expected),
            "missing {expected}: {config}"
        );
    }
    assert_eq!(
        config
            .lines()
            .filter(|line| line.starts_with("identityfile "))
            .count(),
        1
    );
}

#[tokio::test]
async fn cancelled_helpers_terminate_and_output_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let pid_file = root.path().join("pid");
    let pid_path = pid_file.clone();
    let task = tokio::spawn(async move {
        Helper::output(
            Command::new("/bin/sh")
                .args(["-c", "printf '%s' $$ > \"$1\"; exec sleep 30", "helper"])
                .arg(pid_path),
            Duration::from_secs(60),
        )
        .await
    });
    time::timeout(Duration::from_secs(5), async {
        while !pid_file.exists() {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let pid = fs::read_to_string(&pid_file).unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    time::timeout(Duration::from_secs(5), async {
        loop {
            let status = Command::new("/bin/kill")
                .args(["-0", &pid])
                .stderr(Stdio::null())
                .status()
                .await
                .unwrap();
            if !status.success() {
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let output = Helper::output(&mut Command::new("/usr/bin/yes"), HELPER_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(output.stdout.len(), FILE_LIMIT as usize);
    assert!(!output.status.success());
}
