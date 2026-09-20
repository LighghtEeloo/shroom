mod support;

use std::{
    fs,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    process::Command,
};

use shroom_integrations::{
    Attachment, Error, GuestPort, GuestWebUrl, HostKeyHandling, NativeApp, RemoteCommand, SshAlias,
    Terminal, WebApp,
};
use support::Fixture;

#[test]
fn aliases_and_ports_accept_concrete_values_and_reject_patterns_or_options() {
    for alias in ["a", "0", "shroom.project-2_test", &"a".repeat(128)] {
        assert_eq!(alias.parse::<SshAlias>().unwrap().as_str(), alias);
    }
    for alias in [
        "",
        "-oProxyCommand=bad",
        "*.shroom",
        "one two",
        "one\nHost *",
        "é",
        &"a".repeat(129),
    ] {
        assert!(matches!(
            alias.parse::<SshAlias>(),
            Err(Error::InvalidAlias)
        ));
    }
    for port in [1024, 5494, 65535] {
        assert_eq!(GuestPort::try_from(port).unwrap().get(), port as u16);
    }
    for port in [0, 22, 1023, 65536, u32::MAX] {
        assert!(matches!(GuestPort::try_from(port), Err(Error::InvalidGuestPort(p)) if p == port));
    }
}

#[test]
fn attachments_reject_malformed_connections_without_touching_access_files() {
    let fixture = Fixture::new();
    let original = fs::read(&fixture.connection.identity_file).unwrap();
    let pin = fs::read(&fixture.connection.known_hosts_file).unwrap();
    let build = |connection| Attachment::new("test".parse().unwrap(), connection);
    assert!(build(fixture.connection.clone()).is_ok());
    for endpoint in ["0.0.0.0:2222", "192.168.1.2:2222", "127.0.0.1:0"] {
        let mut connection = fixture.connection.clone();
        connection.endpoint = endpoint.parse().unwrap();
        assert!(matches!(
            build(connection),
            Err(Error::InvalidConnection("expected a loopback SSH endpoint"))
        ));
    }
    for user in ["", "-bad", "root\nProxyCommand bad", "developer name"] {
        let mut connection = fixture.connection.clone();
        connection.user = user.into();
        assert!(matches!(
            build(connection),
            Err(Error::InvalidConnection("invalid SSH username"))
        ));
    }
    for path in [
        "relative",
        "~/identity",
        "/tmp/${HOME}",
        "/tmp/key\nIdentityFile /other",
    ] {
        for identity in [true, false] {
            let mut connection = fixture.connection.clone();
            if identity {
                connection.identity_file = path.into();
            } else {
                connection.known_hosts_file = path.into();
            }
            assert!(
                matches!(build(connection), Err(Error::InvalidPath(p)) if p.to_str() == Some(path))
            );
        }
    }
    let mut connection = fixture.connection.clone();
    connection.identity_file = std::ffi::OsString::from_vec(b"/tmp/\xff".to_vec()).into();
    assert!(matches!(build(connection), Err(Error::InvalidPath(_))));
    let other = Fixture::new();
    let mut connection = fixture.connection.clone();
    connection.host_key_alias = other.connection.host_key_alias;
    assert!(matches!(
        build(connection),
        Err(Error::InvalidConnection("host key and alias disagree"))
    ));
    for directory in ["relative", "/tmp/project\n", ""] {
        let mut connection = fixture.connection.clone();
        connection.directory = directory.into();
        assert!(matches!(
            build(connection),
            Err(Error::InvalidConnection(
                "expected an absolute guest directory without control characters"
            ))
        ));
    }
    assert_eq!(
        fs::read(&fixture.connection.identity_file).unwrap(),
        original
    );
    assert_eq!(fs::read(&fixture.connection.known_hosts_file).unwrap(), pin);
}

#[test]
fn openssh_resolves_exported_config_and_commands_with_the_same_trust_policy() {
    let fixture = Fixture::new();
    let attachment = fixture.attachment();
    let config = fixture.config();
    let output = Command::new("ssh")
        .arg("-G")
        .arg("-F")
        .arg(config)
        .arg(attachment.alias().as_str())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config = String::from_utf8(output.stdout).unwrap();
    let command = attachment.command(&RemoteCommand::new("true").unwrap(), Terminal::None);
    let output = Command::new("ssh")
        .arg("-G")
        .args(command.get_args())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "hostname 127.0.0.1".to_owned(),
        "port 2222".to_owned(),
        "user developer".to_owned(),
        "batchmode yes".to_owned(),
        "identityagent none".to_owned(),
        "identitiesonly yes".to_owned(),
        "stricthostkeychecking true".to_owned(),
        "checkhostip no".to_owned(),
        "globalknownhostsfile /dev/null".to_owned(),
        "updatehostkeys false".to_owned(),
        "forwardagent no".to_owned(),
        "forwardx11 no".to_owned(),
        "controlmaster false".to_owned(),
        "hostkeyalgorithms ssh-ed25519".to_owned(),
        format!("hostkeyalias {}", fixture.connection.host_key_alias),
    ] {
        assert!(
            config.lines().any(|line| line == expected),
            "missing {expected}: {config}"
        );
        assert!(
            args.lines().any(|line| line == expected),
            "missing {expected}: {args}"
        );
    }
    for prefix in ["identityfile ", "userknownhostsfile "] {
        let exported = config
            .lines()
            .filter(|line| line.starts_with(prefix))
            .collect::<Vec<_>>();
        let direct = args
            .lines()
            .filter(|line| line.starts_with(prefix))
            .collect::<Vec<_>>();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported, direct);
    }
    assert!(!attachment.ssh_config().contains("RemoteCommand"));
    assert!(!attachment.ssh_config().contains("ClearAllForwardings"));
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "SSH_AUTH_SOCK" && value.is_none())
    );
}

#[test]
fn refreshing_endpoint_preserves_alias_and_trust() {
    let fixture = Fixture::new();
    let old = fixture.attachment();
    let mut connection = fixture.connection.clone();
    connection.endpoint = "[::1]:43210".parse().unwrap();
    let new = Attachment::new(old.alias().clone(), connection).unwrap();
    assert_eq!(old.connection().host_key, new.connection().host_key);
    assert_eq!(
        new.ssh_config(),
        old.ssh_config()
            .replace("127.0.0.1", "::1")
            .replace("2222", "43210")
    );
    assert_eq!(
        old.connection().endpoint.port(),
        2222,
        "snapshots are immutable"
    );
}

#[test]
fn copied_login_command_preserves_shell_literals_and_the_effective_ssh_policy() {
    let fixture = Fixture::new();
    let attachment = fixture.attachment();
    let text = attachment.ssh_command_line();
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(text.replacen("ssh ", "ssh -G ", 1))
        .current_dir(fixture.root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let direct = attachment.command(&RemoteCommand::new("true").unwrap(), Terminal::None);
    let expected = Command::new("ssh")
        .arg("-G")
        .args(direct.get_args())
        .output()
        .unwrap();
    assert!(expected.status.success());
    let actual = String::from_utf8(output.stdout).unwrap();
    let expected = String::from_utf8(expected.stdout).unwrap();
    for prefix in [
        "hostname ",
        "port ",
        "user ",
        "identityfile ",
        "userknownhostsfile ",
        "hostkeyalias ",
        "stricthostkeychecking ",
        "identityagent ",
        "batchmode ",
        "proxycommand ",
        "controlpath ",
        "globalknownhostsfile ",
        "forwardagent ",
    ] {
        assert_eq!(
            actual
                .lines()
                .filter(|line| line.starts_with(prefix))
                .collect::<Vec<_>>(),
            expected
                .lines()
                .filter(|line| line.starts_with(prefix))
                .collect::<Vec<_>>(),
            "{prefix}"
        );
    }
    assert!(!fixture.root.path().join("SHOULD_NOT_EXIST").exists());
}

#[test]
fn shell_quoting_preserves_literal_arguments_and_directory() {
    let fixture = Fixture::new();
    let values = [
        "",
        "two words",
        "a'b\"c",
        "$(touch SHOULD_NOT_EXIST)",
        "`touch SHOULD_NOT_EXIST`",
        "one; two",
        "one\ntwo",
        "$HOME",
        "100%",
    ];
    let remote = values
        .iter()
        .try_fold(
            RemoteCommand::new("/usr/bin/printf")
                .unwrap()
                .with_arg("%s\\0")
                .unwrap(),
            |command, arg| command.with_arg(*arg),
        )
        .unwrap();
    let command = fixture.attachment().command(&remote, Terminal::None);
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command.get_args().last().unwrap())
        .current_dir(fixture.root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        values
            .iter()
            .flat_map(|value| value.bytes().chain([0]))
            .collect::<Vec<_>>()
    );
    assert!(!fixture.root.path().join("SHOULD_NOT_EXIST").exists());
    assert!(
        !std::path::Path::new(&fixture.connection.directory)
            .join("SHOULD_NOT_EXIST")
            .exists()
    );
    let remote = RemoteCommand::new("/bin/pwd").unwrap();
    let command = fixture.attachment().command(&remote, Terminal::None);
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command.get_args().last().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    // macOS /var is a symlink to /private/var.
    assert_eq!(
        fs::canonicalize(String::from_utf8(output.stdout).unwrap().trim()).unwrap(),
        fs::canonicalize(&fixture.connection.directory).unwrap()
    );
}

#[test]
fn bad_commands_and_missing_directories_never_run_the_program() {
    for program in ["kimi", "dsh", "/bin/sh", "my-agent"] {
        assert_eq!(RemoteCommand::new(program).unwrap().program(), program);
    }
    for program in [
        "",
        "-option",
        "./relative",
        "sub/program",
        "echo; touch bad",
        "a\0b",
    ] {
        assert!(matches!(
            RemoteCommand::new(program),
            Err(Error::InvalidProgram)
        ));
    }
    assert!(matches!(
        RemoteCommand::new("echo").unwrap().with_arg("a\0b"),
        Err(Error::InvalidArgument)
    ));
    let fixture = Fixture::new();
    let mut connection = fixture.connection.clone();
    connection.directory.push_str("/does-not-exist");
    let attachment = Attachment::new("test".parse().unwrap(), connection).unwrap();
    let remote = RemoteCommand::new("touch")
        .unwrap()
        .with_arg(fixture.root.path().join("unexpected").to_str().unwrap())
        .unwrap();
    let command = attachment.command(&remote, Terminal::None);
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command.get_args().last().unwrap())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!fixture.root.path().join("unexpected").exists());
}

#[test]
fn web_urls_use_reported_ports_preserve_login_data_and_reject_nonlocal_targets() {
    let fixture = Fixture::new();
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let url = format!("http://{host}:5497/session?token=a%2Fb#login");
        let guest: GuestWebUrl = url.parse().unwrap();
        assert_eq!(guest.as_url().port(), Some(5497));
        let tunnel = fixture
            .attachment()
            .web_tunnel(guest, 15494_u32.try_into().unwrap());
        assert_eq!(
            tunnel.local_url().as_str(),
            "http://127.0.0.1:15494/session?token=a%2Fb#login"
        );
        let command = tunnel.command();
        let expected = if host == "[::1]" {
            "127.0.0.1:15494:[::1]:5497"
        } else {
            "127.0.0.1:15494:127.0.0.1:5497"
        };
        assert!(command.get_args().any(|arg| arg == expected));
        let output = Command::new("ssh")
            .arg("-G")
            .args(command.get_args())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config = String::from_utf8(output.stdout).unwrap();
        assert!(
            config
                .lines()
                .any(|line| line == "exitonforwardfailure yes")
        );
        assert!(config.lines().any(|line| line == "gatewayports no"));
    }
    for url in [
        "https://127.0.0.1:5494",
        "http://127.0.0.1",
        "http://127.0.0.1:22",
        "http://127.0.0.1:0",
        "http://127.0.0.1:65536",
        "http://0.0.0.0:5494",
        "http://192.168.0.2:5494",
        "http://example.com:5494",
        "http://localhost.evil:5494",
        "http://user@127.0.0.1:5494",
        "http://user:password@localhost:5494",
        "http://127.0.0.1:5494\n",
        "not a URL",
    ] {
        assert!(
            matches!(url.parse::<GuestWebUrl>(), Err(Error::InvalidWebUrl)),
            "{url}"
        );
    }
}

#[test]
fn agent_recipes_run_preinstalled_tools_in_the_workspace_without_installers() {
    let fixture = Fixture::new();
    let bin = fixture.root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    for program in ["kimi", "dsh"] {
        let path = bin.join(program);
        let environment_check = if program == "kimi" {
            "test \"$PYTHONUNBUFFERED\" = 1 || exit 42\n"
        } else {
            ""
        };
        fs::write(
            &path,
            format!("#!/bin/sh\n{environment_check}printf '%s\\n' \"$PWD\" \"$@\"\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    // Linux login profiles can reset PATH. Inject the fake tools after profile processing.
    let bash_env = fixture.root.path().join("bash-env");
    fs::write(&bash_env, "export PATH=\"$SHROOM_TEST_BIN:$PATH\"\n").unwrap();
    for app in [WebApp::Kimi, WebApp::DeepSeekHarness] {
        let command = app.launch_command(&fixture.attachment(), app.default_port());
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(command.get_args().last().unwrap())
            .env("BASH_ENV", &bash_env)
            .env("SHROOM_TEST_BIN", &bin)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines = stdout.lines().collect::<Vec<_>>();
        assert_eq!(
            fs::canonicalize(lines[0]).unwrap(),
            fs::canonicalize(&fixture.connection.directory).unwrap()
        );
        assert_eq!(
            &lines[1..],
            [
                "web",
                "--no-open",
                "--host",
                "127.0.0.1",
                "--port",
                &app.default_port().to_string()
            ]
        );
    }
    assert!(
        fixture
            .attachment()
            .kimi_command()
            .get_args()
            .any(|arg| arg == "-tt")
    );
    for app in [
        NativeApp::Codex,
        NativeApp::ClaudeDesktop,
        NativeApp::Cursor,
    ] {
        assert_eq!(app.host_key_handling(), HostKeyHandling::SshConfig);
        assert!(app.documentation().starts_with("https://"));
        assert!(!app.setup().is_empty());
    }
    assert_eq!(
        NativeApp::ZCode.host_key_handling(),
        HostKeyHandling::ClientVerificationRequired
    );
}
