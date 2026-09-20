use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

use super::*;

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        std::os::unix::fs::symlink("/usr/bin/mktemp", bin.join("mktemp")).unwrap();
        std::os::unix::fs::symlink("/bin/rm", bin.join("rm")).unwrap();
        fs::create_dir(root.path().join("tmp")).unwrap();
        let fixture = Self { root };
        fixture.executable(
            "bin/curl",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$HOME/download-args"
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then
        /bin/cp "$HOME/installer" "$2"
        exit "${DOWNLOAD_STATUS:-0}"
    fi
    shift
done
exit 99
"#,
        );
        fs::write(
            fixture.root.path().join("installer"),
            r#"set -eu
[ "$CODEX_NON_INTERACTIVE" = 1 ]
[ "$CODEX_INSTALL_DIR" = "$HOME/.local/bin" ]
/bin/mkdir -p "$CODEX_INSTALL_DIR"
printf '#!/bin/sh\necho codex-test\n' > "$CODEX_INSTALL_DIR/codex"
/bin/chmod +x "$CODEX_INSTALL_DIR/codex"
"#,
        )
        .unwrap();
        fixture
    }

    fn executable(&self, path: &str, contents: &str) {
        let path = self.root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn command(&self, script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .env_clear()
            .env("HOME", self.root.path())
            .env("PATH", self.root.path().join("bin"))
            .env("TMPDIR", self.root.path().join("tmp"));
        command
    }

    fn assert_clean(&self) {
        assert_eq!(
            fs::read_dir(self.root.path().join("tmp")).unwrap().count(),
            0
        );
    }
}

#[tokio::test]
async fn missing_cli_is_installed_without_node_or_root_and_reused() {
    let fixture = Fixture::new();
    Guest::run(fixture.command(Guest::INSTALL), Duration::from_secs(5))
        .await
        .unwrap();
    let args = fs::read_to_string(fixture.root.path().join("download-args")).unwrap();
    assert!(args.contains("https://chatgpt.com/codex/install.sh\n"));
    assert!(args.contains("--proto\n=https\n--proto-redir\n=https\n"));
    fixture.assert_clean();

    // The default guest profile adds .local/bin only after that directory exists.
    let mut verify = fixture.command(Guest::VERIFY);
    verify.env("PATH", fixture.root.path().join(".local/bin"));
    Guest::run(verify, Duration::from_secs(5)).await.unwrap();

    fs::remove_file(fixture.root.path().join("download-args")).unwrap();
    fs::remove_file(fixture.root.path().join("bin/curl")).unwrap();
    Guest::run(fixture.command(Guest::INSTALL), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(!fixture.root.path().join("download-args").exists());
}

#[tokio::test]
async fn existing_working_or_broken_cli_is_never_replaced() {
    for status in [0, 42] {
        let fixture = Fixture::new();
        let original = format!("#!/bin/sh\necho existing-cli >&2\nexit {status}\n");
        fixture.executable("bin/codex", &original);
        let result = Guest::run(fixture.command(Guest::INSTALL), Duration::from_secs(5)).await;
        if status == 0 {
            result.unwrap();
        } else {
            assert!(
                matches!(result, Err(Error::Setup(message)) if message.contains("existing-cli"))
            );
        }
        assert_eq!(
            fs::read_to_string(fixture.root.path().join("bin/codex")).unwrap(),
            original
        );
        assert!(!fixture.root.path().join("download-args").exists());
        assert!(!fixture.root.path().join(".local/bin/codex").exists());
    }
}

#[tokio::test]
async fn failed_download_never_executes_the_partial_installer() {
    let fixture = Fixture::new();
    let mut command = fixture.command(Guest::INSTALL);
    command.env("DOWNLOAD_STATUS", "22");
    assert!(matches!(
        Guest::run(command, Duration::from_secs(5)).await,
        Err(Error::Setup(_))
    ));
    assert!(!fixture.root.path().join(".local/bin/codex").exists());
    fixture.assert_clean();
}

#[tokio::test]
async fn failed_installer_or_missing_binary_is_not_reported_as_ready() {
    for installer in ["echo installer-failed >&2; exit 7", "exit 0"] {
        let fixture = Fixture::new();
        fs::write(fixture.root.path().join("installer"), installer).unwrap();
        let result = Guest::run(fixture.command(Guest::INSTALL), Duration::from_secs(5)).await;
        assert!(matches!(result, Err(Error::Setup(_))));
        assert!(!fixture.root.path().join(".local/bin/codex").exists());
        fixture.assert_clean();
    }
}

#[tokio::test]
async fn installed_cli_outside_login_path_is_rejected_with_recovery_instructions() {
    let fixture = Fixture::new();
    fixture.executable(".local/bin/codex", "#!/bin/sh\nexit 0\n");
    let result = Guest::run(fixture.command(Guest::VERIFY), Duration::from_secs(5)).await;
    assert!(
        matches!(result, Err(Error::Setup(message)) if message.contains("guest login-shell PATH") && message.contains("$HOME/.local/bin"))
    );
}

#[tokio::test]
async fn stuck_process_is_terminated_and_failure_output_is_bounded() {
    let fixture = Fixture::new();
    assert!(matches!(
        Guest::run(
            fixture.command("while :; do :; done"),
            Duration::from_millis(50)
        )
        .await,
        Err(Error::Timeout)
    ));
    let script = "i=0; while [ $i -lt 2000 ]; do echo diagnostic-line >&2; i=$((i + 1)); done; echo final-error >&2; exit 1";
    let result = Guest::run(fixture.command(script), Duration::from_secs(5)).await;
    assert!(
        matches!(result, Err(Error::Setup(message)) if message.ends_with("final-error") && message.len() < 4200)
    );
}

#[test]
fn preparation_uses_the_attachment_trust_and_guest_login_shell() {
    let attachment = crate::codex::Codex::attachment(
        &"alpha".parse().unwrap(),
        crate::test_support::Fixture::connection("alpha"),
    )
    .unwrap();
    let command = Guest::command(&attachment, Guest::INSTALL);
    assert_eq!(Path::new(command.get_program()), Path::new("ssh"));
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>();
    assert_eq!(&args[..2], ["-F", "/dev/null"]);
    assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
    assert!(args.iter().any(|arg| arg == "ClearAllForwardings=yes"));
    assert!(args.last().unwrap().starts_with("exec /bin/bash -lc "));
}
