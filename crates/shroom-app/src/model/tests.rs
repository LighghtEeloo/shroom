use super::*;
use crate::test_support::Fixture as Data;

struct Fixture;

#[test]
fn web_forms_reject_external_urls_and_invalid_ports_without_exposing_tokens() {
    for url in [
        "http://127.0.0.1:5495/path?token=secret#login",
        "http://localhost:3080/",
        "http://[::1]:8080/",
    ] {
        let forward = WebForward::parse(url, "3081").unwrap();
        assert_eq!(forward.local_port.get(), 3081);
        assert!(!format!("{forward:?}").contains("secret"));
    }
    for url in [
        "http://example.com:5494",
        "https://localhost:5494",
        "http://localhost",
        "http://user@localhost:5494",
        "http://127.0.0.1:5494/\n",
    ] {
        assert!(matches!(
            WebForward::parse(url, "3081"),
            Err(Error::Integration(
                shroom_integrations::Error::InvalidWebUrl
            ))
        ));
    }
    for port in ["abc", "-1", "1023", "65536"] {
        assert!(WebForward::parse("http://127.0.0.1:5494/", port).is_err());
    }
    let export = ConnectionExport::with_connection(
        &"project".parse().unwrap(),
        Data::connection("project"),
        ExportFormat::ConnectionDetails,
    )
    .unwrap();
    assert!(export.text.contains("Port: 2222"));
    assert!(export.text.contains("Public host key: ssh-ed25519 "));
    assert!(export.text.contains("Project directory: /workspace"));
}

impl Fixture {
    fn workspace(name: &str, state: SandboxStatus) -> Workspace {
        Data::workspace(name, state)
    }

    fn model() -> Model {
        Model {
            session: Data::session(),
            catalog_current: true,
            image: ImageStatus::Ready,
            selected: Some("alpha".parse().unwrap()),
            workspaces: vec![Self::workspace("alpha", SandboxStatus::Stopped)],
            ..Model::default()
        }
    }

    fn catalog(workspaces: Vec<Workspace>) -> Catalog {
        Catalog {
            workspaces,
            incomplete: Vec::new(),
        }
    }

    fn reply(outcome: Result<Outcome>, workspaces: Vec<Workspace>) -> Reply {
        Reply {
            session: Data::session(),
            selection: None,
            outcome,
            catalog: Ok(Self::catalog(workspaces)),
            image: Ok(ImageStatus::Ready),
        }
    }
}

#[test]
fn ssh_verification_records_proof_while_discovery_and_copying_do_not() {
    let name: WorkspaceName = "alpha".parse().unwrap();
    let running = Data::workspace("alpha", SandboxStatus::Running);
    let mut model = Fixture::model();
    model.begin(&Command::Refresh);
    model.apply(Fixture::reply(Ok(Outcome::Complete), vec![running.clone()]));
    assert!(model.check(&name).is_none());

    model.begin(&Command::Export(name.clone(), ExportFormat::Command));
    model.apply(Fixture::reply(
        Ok(Outcome::Exported(Box::new(
            ConnectionExport::with_connection(
                &name,
                Data::connection("alpha"),
                ExportFormat::Command,
            )
            .unwrap(),
        ))),
        vec![running.clone()],
    ));
    assert!(model.copied.is_some());
    assert!(model.check(&name).is_none());

    for command in [
        Command::Create(name.clone(), 2222.try_into().unwrap(), Default::default()),
        Command::Start(name.clone()),
        Command::Verify(name.clone()),
    ] {
        model.begin(&command);
        model.apply(Fixture::reply(
            Ok(Outcome::Verified(Box::new(Data::connection("alpha")))),
            vec![running.clone()],
        ));
        assert!(matches!(
            model.check(&name).unwrap().result,
            CheckResult::Verified(_)
        ));
        assert_eq!(model.notice.as_deref(), Some(command.success()));
        assert!(model.activity[0].error.is_none());
    }

    model.begin(&Command::Verify(name.clone()));
    assert!(model.check(&name).is_none());
    model.apply(Fixture::reply(Err(Error::SshUnavailable), vec![running]));
    assert!(matches!(
        model.check(&name).unwrap().result,
        CheckResult::Failed
    ));
    assert!(
        model.activity[0]
            .error
            .as_ref()
            .unwrap()
            .contains("SSH details are unavailable")
    );
}

#[test]
fn verification_and_exports_cannot_outlive_the_connection_or_directory() {
    for change in ["endpoint", "stopped", "catalog", "directory"] {
        let name: WorkspaceName = "alpha".parse().unwrap();
        let connection = Data::connection("alpha");
        let mut model = Fixture::model();
        model.begin(&Command::Verify(name.clone()));
        model.apply(Fixture::reply(
            Ok(Outcome::Verified(Box::new(connection.clone()))),
            vec![Data::workspace("alpha", SandboxStatus::Running)],
        ));
        assert!(model.check(&name).is_some());

        model.begin(&Command::Export(name.clone(), ExportFormat::Config));
        let mut workspace = Data::workspace("alpha", SandboxStatus::Running);
        match change {
            "endpoint" => workspace.ssh.as_mut().unwrap().endpoint.set_port(2223),
            "stopped" => {
                workspace.state = SandboxStatus::Stopped;
                workspace.ssh = None;
            }
            _ => (),
        }
        let mut reply = Fixture::reply(
            Ok(Outcome::Exported(Box::new(
                ConnectionExport::with_connection(&name, connection, ExportFormat::Config).unwrap(),
            ))),
            vec![workspace],
        );
        if change == "catalog" {
            reply.catalog = Err(Error::SshUnavailable);
        }
        if change == "directory" {
            reply.session = None;
            reply.catalog = Ok(Catalog::default());
        }
        model.apply(reply);
        assert!(model.check(&name).is_none(), "{change}");
        assert!(model.copied.is_none(), "{change}");
    }
}

#[test]
fn incomplete_selection_and_diagnostics_survive_failed_cleanup() {
    let name: WorkspaceName = "partial".parse().unwrap();
    let mut model = Fixture::model();
    model.select(name.clone());
    model.begin(&Command::Cleanup(name.clone()));
    let mut reply = Fixture::reply(Err(Error::SshUnavailable), model.workspaces.clone());
    reply
        .catalog
        .as_mut()
        .unwrap()
        .incomplete
        .push(name.clone());
    model.apply(reply);
    assert_eq!(model.selected_incomplete(), Some(&name));
    assert!(model.available());
    assert_eq!(model.activity[0].command.target(), Some(&name));
    assert_eq!(
        model.activity[0].error.as_deref(),
        Some(Error::SshUnavailable.to_string().as_str())
    );
}

#[test]
fn create_parses_domain_types_and_rejects_bad_names_and_ports() {
    for port in ["1024", "65535"] {
        let command = CreateForm {
            name: "project-1".into(),
            port: port.into(),
            ..Default::default()
        }
        .parse()
        .unwrap();
        assert!(
            matches!(command, Command::Create(name, parsed, _) if name.as_str() == "project-1" && parsed.to_string() == port)
        );
    }
    for name in ["", "Project", "../project", "-project", "two words"] {
        assert!(matches!(
            CreateForm {
                name: name.into(),
                port: "2222".into(),
                ..Default::default()
            }
            .parse(),
            Err(Error::Core(shroom_core::Error::InvalidName))
        ));
    }
    for port in ["0", "1023", "65536"] {
        assert!(matches!(
            CreateForm {
                name: "project".into(),
                port: port.into(),
                ..Default::default()
            }
            .parse(),
            Err(Error::Core(shroom_core::Error::InvalidPort(_)))
        ));
    }
    for port in ["", "22.5", "ssh", "-1", "4294967296"] {
        assert!(matches!(
            CreateForm {
                name: "project".into(),
                port: port.into(),
                ..Default::default()
            }
            .parse(),
            Err(Error::InvalidPort)
        ));
    }
}

#[test]
fn setup_preserves_literal_paths_and_rejects_ambiguous_paths() {
    let valid = SetupForm {
        state_dir: "/tmp/shroom state %".into(),
        runtime: "/opt/runtime/msb".into(),
        firmware: "/opt/runtime/libkrunfw".into(),
    };
    assert!(
        matches!(valid.parse().unwrap(), Command::Open { state_dir, config } if state_dir == std::path::Path::new(&valid.state_dir) && config.runtime_executable == std::path::Path::new(&valid.runtime))
    );
    for path in [
        "",
        "relative",
        "~/.shroom",
        "/tmp/$HOME",
        "/tmp/line\nbreak",
    ] {
        let invalid = SetupForm {
            state_dir: path.into(),
            ..valid.clone()
        };
        assert!(matches!(
            invalid.parse(),
            Err(Error::InvalidPath("State directory"))
        ));
    }
    assert!(matches!(
        SetupForm {
            runtime: "msb".into(),
            ..valid.clone()
        }
        .parse(),
        Err(Error::InvalidPath("Runtime executable"))
    ));
    assert!(matches!(
        SetupForm {
            firmware: "".into(),
            ..valid
        }
        .parse(),
        Err(Error::InvalidPath("Firmware"))
    ));
}

#[test]
fn busy_model_rejects_duplicate_operations_and_invalidates_old_exports() {
    let mut model = Fixture::model();
    model.copied = Some(
        ConnectionExport::with_connection(
            &"alpha".parse().unwrap(),
            Data::connection("alpha"),
            ExportFormat::Command,
        )
        .unwrap(),
    );
    model.confirm_remove = model.selected.clone();
    assert!(model.begin(&Command::Start("alpha".parse().unwrap())));
    assert!(!model.available());
    assert!(model.copied.is_none());
    assert!(model.confirm_remove.is_none());
    assert!(!model.begin(&Command::Remove("alpha".parse().unwrap())));
    assert!(matches!(model.pending, Some(Command::Start(_))));
}

#[test]
fn failed_mutation_still_shows_partial_artifacts_and_clears_busy_state() {
    let mut model = Fixture::model();
    model.begin(&Command::Create(
        "partial".parse().unwrap(),
        2222.try_into().unwrap(),
        Default::default(),
    ));
    model.apply(Reply {
        session: Data::session(),
        selection: Some("partial".parse().unwrap()),
        outcome: Err(Error::SshUnavailable),
        catalog: Ok(Fixture::catalog(vec![Fixture::workspace(
            "partial",
            SandboxStatus::Running,
        )])),
        image: Ok(ImageStatus::Ready),
    });
    assert!(model.available());
    assert_eq!(model.workspace().unwrap().name.as_str(), "partial");
    assert_eq!(model.errors, [Error::SshUnavailable.to_string()]);
    assert!(model.copied.is_none());
}

#[test]
fn catalog_failure_disables_stale_actions_until_refresh_succeeds() {
    let mut model = Fixture::model();
    model.apply(Reply {
        session: Data::session(),
        selection: None,
        outcome: Ok(Outcome::Exported(Box::new(
            ConnectionExport::with_connection(
                &"alpha".parse().unwrap(),
                Data::connection("alpha"),
                ExportFormat::Command,
            )
            .unwrap(),
        ))),
        catalog: Err(shroom_core::Error::AccessIncomplete("/tmp/access".into()).into()),
        image: Ok(ImageStatus::Ready),
    });
    assert!(!model.available());
    assert_eq!(model.workspaces.len(), 1);
    assert!(model.copied.is_none());
    assert!(model.errors[0].contains("Could not refresh workspaces"));
    assert!(model.begin(&Command::Refresh));
    model.apply(Reply {
        session: Data::session(),
        selection: None,
        outcome: Ok(Outcome::Complete),
        catalog: Ok(Catalog::default()),
        image: Ok(ImageStatus::Ready),
    });
    assert!(model.available());
    assert!(model.selected.is_none());
    assert!(model.errors.is_empty());
}

#[test]
fn selection_and_removal_cannot_reuse_another_workspaces_confirmation() {
    let mut model = Fixture::model();
    model.confirm_remove = model.selected.clone();
    model.copied = Some(
        ConnectionExport::with_connection(
            &"alpha".parse().unwrap(),
            Data::connection("alpha"),
            ExportFormat::Config,
        )
        .unwrap(),
    );
    model.select("beta".parse().unwrap());
    assert!(model.confirm_remove.is_none());
    assert!(model.copied.is_none());
    model.apply(Reply {
        session: Data::session(),
        selection: None,
        outcome: Ok(Outcome::Complete),
        catalog: Ok(Fixture::catalog(vec![Fixture::workspace(
            "alpha",
            SandboxStatus::Stopped,
        )])),
        image: Ok(ImageStatus::Ready),
    });
    assert_eq!(model.workspace().unwrap().name.as_str(), "alpha");
}

#[test]
fn lifecycle_actions_require_appropriate_native_states() {
    assert!(Actions::start(SandboxStatus::Stopped));
    assert!(Actions::start(SandboxStatus::Running));
    assert!(Actions::stop(SandboxStatus::Running));
    assert!(Actions::remove(SandboxStatus::Stopped));
    assert!(Actions::remove(SandboxStatus::Crashed));
    assert!(!Actions::stop(SandboxStatus::Stopped));
    for state in [
        SandboxStatus::Starting,
        SandboxStatus::Draining,
        SandboxStatus::Paused,
    ] {
        assert!(!Actions::start(state));
        assert!(!Actions::remove(state));
    }
    assert!(!Actions::remove(SandboxStatus::Running));
}

#[test]
fn image_architecture_must_match_the_host() {
    let (matching, incompatible) = if cfg!(target_arch = "aarch64") {
        ("arm64", "amd64")
    } else {
        ("amd64", "arm64")
    };
    Session::check_architecture(Some(matching)).unwrap();
    for architecture in [None, Some(incompatible), Some("unknown")] {
        assert!(matches!(Session::check_architecture(architecture),
            Err(Error::ImageArchitecture(actual)) if actual.as_deref() == architecture));
    }
}

#[tokio::test]
async fn disconnected_requests_fail_without_acquiring_a_core() {
    let backend = Backend::default();
    let reply = backend.execute(Command::Refresh).await;
    assert!(matches!(reply.outcome, Err(Error::Disconnected)));
    assert!(reply.session.is_none());
    assert!(reply.catalog.unwrap().workspaces.is_empty());
    let reply = backend
        .execute(Command::AddToCodex("alpha".parse().unwrap()))
        .await;
    assert!(matches!(reply.outcome, Err(Error::Disconnected)));
    assert!(reply.session.is_none());
    let reply = backend.execute(Command::Disconnect).await;
    assert!(reply.outcome.is_ok());
    assert!(reply.session.is_none());
}

#[tokio::test]
async fn failed_open_releases_the_directory_lock_for_retry() {
    let root = tempfile::tempdir().unwrap();
    let backend = Backend::default();
    let command = Command::Open {
        state_dir: root.path().into(),
        config: Config {
            runtime_executable: root.path().join("missing-msb"),
            firmware: root.path().join("missing-firmware"),
        },
    };
    for _ in 0..2 {
        let reply = backend.execute(command.clone()).await;
        assert!(reply.session.is_none());
        assert!(matches!(reply.outcome, Err(Error::Core(_))));
        assert!(!matches!(
            reply.outcome,
            Err(Error::Core(shroom_core::Error::CoreInUse(_)))
        ));
        assert!(reply.catalog.unwrap().workspaces.is_empty());
        assert!(!root.path().join("access").exists());
    }
}

#[tokio::test]
#[ignore = "requires the pinned runtime, a prepared image archive, virtualization, and a free port"]
async fn missing_image_recovery_and_workspace_lifecycle() {
    let state_dir = tempfile::Builder::new()
        .prefix("sh-app-")
        .tempdir_in("/tmp")
        .unwrap()
        .keep();
    eprintln!("app acceptance state: {}", state_dir.display());
    let backend = Backend::default();
    let reply = backend
        .execute(Command::Open {
            state_dir: state_dir.clone(),
            config: Config {
                runtime_executable: env::var_os("SHROOM_TEST_RUNTIME")
                    .expect("set SHROOM_TEST_RUNTIME")
                    .into(),
                firmware: env::var_os("SHROOM_TEST_FIRMWARE")
                    .expect("set SHROOM_TEST_FIRMWARE")
                    .into(),
            },
        })
        .await;
    reply.outcome.unwrap();
    assert_eq!(reply.image.unwrap(), ImageStatus::Missing);
    let name: WorkspaceName = "recovery".parse().unwrap();
    let port = env::var("SHROOM_APP_TEST_PORT")
        .unwrap_or_else(|_| "34882".into())
        .parse::<u32>()
        .unwrap()
        .try_into()
        .unwrap();
    let reply = backend
        .execute(Command::Create(name.clone(), port, Default::default()))
        .await;
    assert!(matches!(reply.outcome, Err(Error::ImageMissing)));
    assert!(reply.catalog.unwrap().incomplete.is_empty());
    assert!(!state_dir.join("access/recovery").exists());

    // Reproduce the previous app's create path, which left access files before image resolution failed.
    {
        let mut slot = backend.session.lock().await;
        let error = slot
            .as_mut()
            .unwrap()
            .core
            .create(name.clone(), port, Default::default())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            shroom_core::Error::Operation {
                stage: shroom_core::Stage::CreateSandbox,
                ..
            }
        ));
    }
    let reply = backend.execute(Command::Refresh).await;
    let catalog = reply.catalog.unwrap();
    assert!(catalog.workspaces.is_empty());
    assert_eq!(catalog.incomplete.as_slice(), std::slice::from_ref(&name));
    let reply = backend.execute(Command::Cleanup(name.clone())).await;
    reply.outcome.unwrap();
    assert!(reply.catalog.unwrap().incomplete.is_empty());
    assert!(!state_dir.join("access/recovery").exists());

    let invalid = state_dir.join("invalid.tar");
    fs::write(&invalid, b"not an image archive").unwrap();
    let reply = backend.execute(Command::ImportImage(invalid)).await;
    assert!(matches!(reply.outcome, Err(Error::Sdk(_))));
    assert_eq!(reply.image.unwrap(), ImageStatus::Missing);
    let archive = env::var_os("SHROOM_TEST_IMAGE_ARCHIVE")
        .expect("set SHROOM_TEST_IMAGE_ARCHIVE")
        .into();
    let reply = backend.execute(Command::ImportImage(archive)).await;
    reply.outcome.unwrap();
    assert_eq!(reply.image.unwrap(), ImageStatus::Ready);

    let reply = backend
        .execute(Command::Create(name.clone(), port, Default::default()))
        .await;
    reply.outcome.unwrap();
    let catalog = reply.catalog.unwrap();
    let connection = catalog.workspaces[0].ssh.clone().unwrap();
    assert_eq!(catalog.workspaces[0].state, SandboxStatus::Running);
    let reply = backend.execute(Command::Cleanup(name.clone())).await;
    assert!(matches!(reply.outcome, Err(Error::WorkspaceExists(_))));
    assert!(connection.identity_file.is_file());
    assert_eq!(
        reply.catalog.unwrap().workspaces[0].state,
        SandboxStatus::Running
    );
    let reply = backend
        .execute(Command::Export(name.clone(), ExportFormat::Config))
        .await;
    let Outcome::Exported(export) = reply.outcome.unwrap() else {
        panic!("expected SSH export");
    };
    let config = export.text;
    assert!(config.contains("StrictHostKeyChecking yes"));
    assert!(config.contains(&connection.host_key_alias.to_string()));
    let reply = backend.execute(Command::Verify(name.clone())).await;
    assert!(matches!(reply.outcome, Ok(Outcome::Verified(_))));
    let reply = backend
        .execute(Command::Export(name.clone(), ExportFormat::Command))
        .await;
    let Outcome::Exported(export) = reply.outcome.unwrap() else {
        panic!("expected command export");
    };
    let output = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{} 'id -un'", export.text))
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "developer"
    );
    backend
        .execute(Command::Stop(name.clone()))
        .await
        .outcome
        .unwrap();
    let reply = backend.execute(Command::Verify(name.clone())).await;
    assert!(matches!(reply.outcome, Err(Error::SshUnavailable)));
    assert_eq!(
        reply.catalog.unwrap().workspaces[0].state,
        SandboxStatus::Stopped
    );
    let reply = backend.execute(Command::Start(name.clone())).await;
    reply.outcome.unwrap();
    assert_eq!(
        reply.catalog.unwrap().workspaces[0]
            .ssh
            .as_ref()
            .unwrap()
            .host_key,
        connection.host_key
    );
    backend
        .execute(Command::Stop(name.clone()))
        .await
        .outcome
        .unwrap();
    let reply = backend.execute(Command::Remove(name)).await;
    reply.outcome.unwrap();
    let catalog = reply.catalog.unwrap();
    assert!(catalog.workspaces.is_empty());
    assert!(catalog.incomplete.is_empty());
    backend.execute(Command::Disconnect).await.outcome.unwrap();
    fs::remove_dir_all(state_dir).unwrap();
}

#[test]
fn create_carries_custom_user_and_each_folder_access_mode() {
    let host = tempfile::tempdir().unwrap();
    let mut form = CreateForm {
        name: "project".into(),
        user: "arctic".into(),
        folders: vec![
            FolderForm {
                host: host.path().display().to_string(),
                guest: "/mnt/source".into(),
                writable: true,
            },
            FolderForm {
                host: host.path().display().to_string(),
                guest: "/mnt/reference".into(),
                writable: false,
            },
        ],
        ..Default::default()
    };
    let Command::Create(_, _, options) = form.parse().unwrap() else {
        panic!("expected create")
    };
    assert_eq!(options.user.as_str(), "arctic");
    assert_eq!(options.folders[0].access(), FolderAccess::ReadWrite);
    assert_eq!(options.folders[1].access(), FolderAccess::ReadOnly);
    form.user = "root".into();
    assert!(matches!(
        form.parse(),
        Err(Error::Core(shroom_core::Error::InvalidGuestUser))
    ));
    form.user = "arctic".into();
    form.folders[0].guest = "/etc/ssh".into();
    assert!(matches!(
        form.parse(),
        Err(Error::Core(shroom_core::Error::InvalidHostFolder(_)))
    ));
    assert_eq!(std::fs::read_dir(host.path()).unwrap().count(), 0);
}
