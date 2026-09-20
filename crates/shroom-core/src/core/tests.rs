use super::*;

#[tokio::test]
async fn lock_conflict_preserves_existing_state_and_failure_releases_lock() {
    let root = tempfile::tempdir().unwrap();
    let lock = StateLock::new(root.path()).unwrap();
    let sentinel = root.path().join("existing");
    fs::write(&sentinel, "unchanged").unwrap();
    let config = Config {
        runtime_executable: root.path().join("absent"),
        firmware: root.path().join("absent"),
    };
    assert!(matches!(
        Core::open(root.path().to_owned(), config.clone()).await,
        Err(Error::CoreInUse(_))
    ));
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "unchanged");
    assert!(!root.path().join("microsandbox").exists());
    drop(lock);
    let error = Core::open(root.path().to_owned(), config)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&error, Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound),
        "{error:?}"
    );
    let lock = File::open(root.path().join("core.lock")).unwrap();
    lock.try_lock().unwrap();
}

#[tokio::test]
async fn native_profile_rejects_conflicting_settings_before_creation() {
    let root = tempfile::tempdir().unwrap();
    let backend = LocalBackend::builder()
        .home(root.path())
        .config_path(root.path().join("config.json"))
        .build_lazy()
        .unwrap();
    let config = microsandbox::with_backend(
        Arc::new(backend) as Arc<dyn Backend>,
        Core::sandbox_config(
            &"profile".parse().unwrap(),
            HostPort::try_from(2222).unwrap(),
        ),
    )
    .await
    .unwrap();
    Core::validate_config(&config).unwrap();
    let conflicts: [fn(&mut SandboxConfig); 8] = [
        |c| c.spec.lifecycle.ephemeral = true,
        |c| c.spec.lifecycle.idle_timeout_secs = Some(5),
        |c| {
            c.spec.image = RootfsSource::Bind {
                path: "/tmp".into(),
                follow_root_symlinks: false,
            }
        },
        |c| {
            c.spec.env.push(microsandbox::sandbox::EnvVar {
                key: "TOKEN".into(),
                value: "do-not-copy".into(),
            })
        },
        |c| c.spec.network.trust_host_cas = true,
        |c| c.spec.network.ports[0].host_bind = "0.0.0.0".into(),
        |c| c.spec.network.policy = None,
        |c| c.spec.resources.cpus = 8,
    ];
    for conflict in conflicts {
        let mut changed = config.clone();
        conflict(&mut changed);
        assert!(matches!(
            Core::validate_config(&changed),
            Err(Error::Sdk(MicrosandboxError::InvalidConfig(_)))
        ));
    }
    assert!(!root.path().join("sandboxes").exists());
}

#[tokio::test]
async fn unrecognized_runtime_is_rejected_before_catalog_initialization() {
    let root = tempfile::tempdir().unwrap();
    let firmware = root.path().join("firmware");
    fs::write(&firmware, "not used: executable validation fails first").unwrap();
    let config = Config {
        runtime_executable: "/bin/sh".into(),
        firmware,
    };
    let error = Core::open(root.path().to_owned(), config)
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        Error::Sdk(MicrosandboxError::InvalidConfig(_))
    ));
    assert!(!root.path().join("microsandbox/db").exists());
    StateLock::new(root.path()).unwrap();
}
