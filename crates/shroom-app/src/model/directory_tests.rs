use std::{
    process::{Output, Stdio},
    time::Duration,
};

use shroom_integrations::{RemoteCommand, Terminal};
use tokio::io::AsyncWriteExt;

use super::*;

struct Runtime;

impl Runtime {
    async fn execute(backend: &Backend, command: Command) -> Reply {
        Box::pin(backend.execute(command)).await
    }

    async fn output(command: std::process::Command) -> Output {
        tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::from(command)
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
            .try_fold(RemoteCommand::new(program).unwrap(), |command, arg| {
                command.with_arg(*arg)
            })
            .unwrap();
        Self::output(attachment.login_command(&remote, Terminal::None)).await
    }

    async fn project(backend: &Backend, name: &WorkspaceName) -> Project {
        backend
            .session
            .lock()
            .await
            .as_ref()
            .unwrap()
            .project(name)
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires the pinned runtime, prepared image, virtualization, and a free SSH port"]
async fn directory_preferences_persist_and_validate_in_a_real_guest() {
    let root = tempfile::Builder::new()
        .prefix("sd-")
        .tempdir_in("/tmp")
        .unwrap()
        .keep();
    eprintln!("working directory acceptance state: {}", root.display());
    let host = root.join("shared");
    fs::create_dir(&host).unwrap();
    fs::write(host.join("sentinel"), "preserve").unwrap();
    let state = root.join("state");
    let config = Config {
        runtime_executable: env::var_os("SHROOM_TEST_RUNTIME")
            .expect("set SHROOM_TEST_RUNTIME")
            .into(),
        firmware: env::var_os("SHROOM_TEST_FIRMWARE")
            .expect("set SHROOM_TEST_FIRMWARE")
            .into(),
    };
    let backend = Backend::default();
    Runtime::execute(
        &backend,
        Command::Open {
            state_dir: state.clone(),
            config: config.clone(),
        },
    )
    .await
    .outcome
    .unwrap();
    Runtime::execute(
        &backend,
        Command::ImportImage(
            env::var_os("SHROOM_TEST_IMAGE_ARCHIVE")
                .expect("set SHROOM_TEST_IMAGE_ARCHIVE")
                .into(),
        ),
    )
    .await
    .outcome
    .unwrap();
    let name: WorkspaceName = "directory".parse().unwrap();
    let port = env::var("SHROOM_DIRECTORY_TEST_PORT")
        .unwrap_or_else(|_| "35182".into())
        .parse::<u32>()
        .unwrap()
        .try_into()
        .unwrap();
    let options = WorkspaceOptions {
        user: "arctic".parse().unwrap(),
        folders: vec![
            HostFolder::new(host.clone(), "/mnt/project".into(), FolderAccess::ReadOnly).unwrap(),
        ],
    };
    Runtime::execute(
        &backend,
        Command::Create(name.clone(), port, options.clone()),
    )
    .await
    .outcome
    .unwrap();
    let project = Runtime::project(&backend, &name).await;
    assert_eq!(project.directory.as_str(), "/home/arctic/workspace");
    let attachment = project.attachment;
    let chosen: GuestDirectory = "/home/arctic/workspace/project's files".parse().unwrap();
    assert!(
        Runtime::run(
            &attachment,
            "mkdir",
            &["-p", chosen.as_str(), "/home/arctic/volatile"]
        )
        .await
        .status
        .success()
    );
    assert!(
        Runtime::run(
            &attachment,
            "ln",
            &["-s", "/does-not-exist", "/home/arctic/broken-link"]
        )
        .await
        .status
        .success()
    );
    Runtime::execute(
        &backend,
        Command::SetWorkingDirectory(name.clone(), chosen.clone()),
    )
    .await
    .outcome
    .unwrap();
    let project = Runtime::project(&backend, &name).await;
    ProjectCheck::check(&project).await.unwrap();
    let pwd = project.command(&RemoteCommand::new("pwd").unwrap(), Terminal::None);
    assert_eq!(
        String::from_utf8(Runtime::output(pwd).await.stdout)
            .unwrap()
            .trim(),
        chosen.as_str()
    );

    // Execute the exported command through a host shell, with a real guest PTY and interactive Bash.
    let mut terminal = tokio::process::Command::new("/bin/sh")
        .args(["-c", &project.terminal_command_line()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    terminal
        .stdin
        .take()
        .unwrap()
        .write_all(b"printf 'SHROOM_CWD='; pwd; exit\n")
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(15), terminal.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("SHROOM_CWD={chosen}")));

    for path in [
        "/root",
        "/bin/bash",
        "/home/arctic/broken-link",
        "/does-not-exist",
    ] {
        Runtime::execute(
            &backend,
            Command::SetWorkingDirectory(name.clone(), path.parse().unwrap()),
        )
        .await
        .outcome
        .unwrap();
        let rejected = Runtime::project(&backend, &name).await;
        assert!(matches!(
            ProjectCheck::check(&rejected).await,
            Err(crate::project::Error::DirectoryUnavailable(_))
        ));
        let remote = RemoteCommand::new("touch")
            .unwrap()
            .with_arg("/home/arctic/should-not-exist")
            .unwrap();
        assert_eq!(
            Runtime::output(rejected.command(&remote, Terminal::None))
                .await
                .status
                .code(),
            Some(Project::DIRECTORY_UNAVAILABLE)
        );
    }
    assert!(
        !Runtime::run(
            &attachment,
            "test",
            &["-e", "/home/arctic/should-not-exist"]
        )
        .await
        .status
        .success()
    );
    Runtime::execute(
        &backend,
        Command::SetWorkingDirectory(name.clone(), "/mnt/project".parse().unwrap()),
    )
    .await
    .outcome
    .unwrap();
    ProjectCheck::check(&Runtime::project(&backend, &name).await)
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(host.join("sentinel")).unwrap(),
        "preserve"
    );

    // A fixture CLI proves initial failure precedes preparation and detects a directory lost during it.
    let install = r#"mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/codex" <<'SCRIPT'
#!/bin/sh
echo checked >> "$HOME/codex-checks"
rmdir "$HOME/volatile" 2>/dev/null || true
echo codex-fixture
SCRIPT
chmod +x "$HOME/.local/bin/codex"
printf '%s\n' 'export PATH="$HOME/.local/bin:$PATH"' > "$HOME/.bash_profile"
"#;
    assert!(
        Runtime::run(&attachment, "/bin/sh", &["-c", install])
            .await
            .status
            .success()
    );
    let missing = Project {
        attachment: attachment.clone(),
        directory: "/missing".parse().unwrap(),
    };
    assert!(matches!(
        crate::codex::Codex::prepare_project(&missing).await,
        Err(crate::codex::Error::Project(
            crate::project::Error::DirectoryUnavailable(_)
        ))
    ));
    assert!(
        !Runtime::run(&attachment, "test", &["-e", "/home/arctic/codex-checks"])
            .await
            .status
            .success()
    );
    let volatile = Project {
        attachment: attachment.clone(),
        directory: "/home/arctic/volatile".parse().unwrap(),
    };
    assert!(matches!(
        crate::codex::Codex::prepare_project(&volatile).await,
        Err(crate::codex::Error::Project(
            crate::project::Error::DirectoryUnavailable(_)
        ))
    ));
    assert!(
        Runtime::run(&attachment, "test", &["-s", "/home/arctic/codex-checks"])
            .await
            .status
            .success()
    );
    crate::codex::Codex::prepare_project(&Runtime::project(&backend, &name).await)
        .await
        .unwrap();

    Runtime::execute(&backend, Command::Stop(name.clone()))
        .await
        .outcome
        .unwrap();
    let reply = Runtime::execute(
        &backend,
        Command::SetWorkingDirectory(name.clone(), chosen.clone()),
    )
    .await;
    reply.outcome.unwrap();
    let workspace = &reply.catalog.unwrap().workspaces[0];
    assert_eq!(workspace.state, SandboxStatus::Stopped);
    assert_eq!(workspace.directory().unwrap(), chosen);
    Runtime::execute(&backend, Command::Disconnect)
        .await
        .outcome
        .unwrap();
    let reply = Runtime::execute(
        &backend,
        Command::Open {
            state_dir: state.clone(),
            config,
        },
    )
    .await;
    reply.outcome.unwrap();
    assert_eq!(
        reply.catalog.unwrap().workspaces[0].directory().unwrap(),
        chosen
    );
    Runtime::execute(&backend, Command::Start(name.clone()))
        .await
        .outcome
        .unwrap();
    assert_eq!(Runtime::project(&backend, &name).await.directory, chosen);

    let file = state.join("access/directory/app/launch.json");
    fs::write(&file, "corrupt").unwrap();
    let reply = Runtime::execute(&backend, Command::Refresh).await;
    reply.outcome.unwrap();
    assert!(reply.catalog.unwrap().workspaces[0].directory().is_err());
    Runtime::execute(&backend, Command::Verify(name.clone()))
        .await
        .outcome
        .unwrap();
    Runtime::execute(
        &backend,
        Command::Export(name.clone(), ExportFormat::HomeLogin),
    )
    .await
    .outcome
    .unwrap();
    assert!(
        Runtime::execute(
            &backend,
            Command::Export(name.clone(), ExportFormat::Command)
        )
        .await
        .outcome
        .is_err()
    );
    Runtime::execute(&backend, Command::ResetWorkingDirectory(name.clone()))
        .await
        .outcome
        .unwrap();
    assert!(!file.exists());
    Runtime::execute(&backend, Command::SetWorkingDirectory(name.clone(), chosen))
        .await
        .outcome
        .unwrap();
    Runtime::execute(&backend, Command::Stop(name.clone()))
        .await
        .outcome
        .unwrap();
    Runtime::execute(&backend, Command::Remove(name.clone()))
        .await
        .outcome
        .unwrap();
    assert!(!file.exists());
    Runtime::execute(&backend, Command::Create(name.clone(), port, options))
        .await
        .outcome
        .unwrap();
    assert_eq!(
        Runtime::project(&backend, &name).await.directory.as_str(),
        "/home/arctic/workspace"
    );
    Runtime::execute(&backend, Command::Stop(name.clone()))
        .await
        .outcome
        .unwrap();
    Runtime::execute(&backend, Command::Remove(name))
        .await
        .outcome
        .unwrap();
    Runtime::execute(&backend, Command::Disconnect)
        .await
        .outcome
        .unwrap();
    assert_eq!(
        fs::read_to_string(host.join("sentinel")).unwrap(),
        "preserve"
    );
    fs::remove_dir_all(root).unwrap();
}
