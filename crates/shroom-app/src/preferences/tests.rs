use super::*;
use std::os::unix::fs::symlink;

struct Fixture {
    root: tempfile::TempDir,
    store: PreferenceStore,
    name: WorkspaceName,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = PreferenceStore {
            state_dir: root.path().into(),
        };
        let name = "alpha".parse().unwrap();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root.path().join("access/alpha"))
            .unwrap();
        Self { root, store, name }
    }

    fn custom(&self) -> WorkingDirectoryPreference {
        WorkingDirectoryPreference::Custom("/mnt/用户 project's $files/".parse().unwrap())
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("access/alpha/app/launch.json")
    }

    fn save(&self) {
        self.store.save(&self.name, &self.custom()).unwrap();
    }
}

#[test]
fn preferences_persist_reset_and_follow_the_access_directory_lifetime() {
    let fixture = Fixture::new();
    let user = "arctic".parse().unwrap();
    assert_eq!(
        fixture
            .store
            .load(&fixture.name)
            .unwrap()
            .resolve(&user)
            .as_str(),
        "/home/arctic/workspace"
    );
    fixture
        .store
        .save(&fixture.name, &WorkingDirectoryPreference::Default)
        .unwrap();
    assert!(!fixture.path().parent().unwrap().exists());
    fixture.save();
    let reopened = PreferenceStore {
        state_dir: fixture.root.path().into(),
    };
    assert_eq!(reopened.load(&fixture.name).unwrap(), fixture.custom());
    assert_eq!(
        fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(fixture.path().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let other = Fixture::new();
    assert_eq!(
        other.store.load(&other.name).unwrap(),
        WorkingDirectoryPreference::Default
    );
    fixture
        .store
        .save(&fixture.name, &WorkingDirectoryPreference::Default)
        .unwrap();
    assert_eq!(
        reopened.load(&fixture.name).unwrap(),
        WorkingDirectoryPreference::Default
    );
    fixture.save();
    fs::remove_dir_all(fixture.root.path().join("access/alpha")).unwrap();
    assert!(
        fixture
            .store
            .save(&fixture.name, &fixture.custom())
            .is_err()
    );
    assert!(!fixture.root.path().join("access/alpha").exists());
    fs::DirBuilder::new()
        .mode(0o700)
        .create(fixture.root.path().join("access/alpha"))
        .unwrap();
    assert_eq!(
        reopened.load(&fixture.name).unwrap(),
        WorkingDirectoryPreference::Default
    );
}

#[test]
fn invalid_records_preserve_bytes_until_explicit_replacement_or_reset() {
    let fixture = Fixture::new();
    fixture.save();
    let malformed = b"broken";
    fs::write(fixture.path(), malformed).unwrap();
    assert!(matches!(
        fixture.store.load(&fixture.name),
        Err(Error::InvalidRecord(_))
    ));
    assert_eq!(fs::read(fixture.path()).unwrap(), malformed);
    let unsupported = br#"{"version":9,"working_directory":"/mnt/project"}"#;
    fs::write(fixture.path(), unsupported).unwrap();
    assert!(matches!(
        fixture.store.load(&fixture.name),
        Err(Error::UnsupportedVersion(9))
    ));
    assert_eq!(fs::read(fixture.path()).unwrap(), unsupported);
    let relative = br#"{"version":1,"working_directory":"relative"}"#;
    fs::write(fixture.path(), relative).unwrap();
    assert!(matches!(
        fixture.store.load(&fixture.name),
        Err(Error::InvalidDirectory(_))
    ));
    assert_eq!(fs::read(fixture.path()).unwrap(), relative);
    fs::write(
        fixture.path(),
        vec![b'x'; PreferenceStore::LIMIT as usize + 1],
    )
    .unwrap();
    assert!(matches!(
        fixture.store.load(&fixture.name),
        Err(Error::TooLarge)
    ));
    fixture
        .store
        .save(&fixture.name, &WorkingDirectoryPreference::Default)
        .unwrap();
    fixture.save();
    let before = fs::read(fixture.path()).unwrap();
    let oversized = WorkingDirectoryPreference::Custom(
        format!("/{}", "x".repeat(PreferenceStore::LIMIT as usize))
            .parse()
            .unwrap(),
    );
    assert!(matches!(
        fixture.store.save(&fixture.name, &oversized),
        Err(Error::TooLarge)
    ));
    assert_eq!(fs::read(fixture.path()).unwrap(), before);
}

#[test]
fn unsafe_storage_entries_never_redirect_writes_or_reset() {
    let fixture = Fixture::new();
    fixture.save();
    let outside = fixture.root.path().join("outside");
    fs::write(&outside, "preserve").unwrap();
    fs::remove_file(fixture.path()).unwrap();
    symlink(&outside, fixture.path()).unwrap();
    assert!(matches!(
        fixture.store.load(&fixture.name),
        Err(Error::UnsafePath(_))
    ));
    for choice in [fixture.custom(), WorkingDirectoryPreference::Default] {
        assert!(matches!(
            fixture.store.save(&fixture.name, &choice),
            Err(Error::UnsafePath(_))
        ));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "preserve");
    }
    fs::remove_file(fixture.path()).unwrap();
    fs::remove_dir(fixture.path().parent().unwrap()).unwrap();
    symlink(fixture.root.path(), fixture.path().parent().unwrap()).unwrap();
    assert!(matches!(
        fixture.store.save(&fixture.name, &fixture.custom()),
        Err(Error::UnsafePath(_))
    ));
    assert!(!fixture.root.path().join("launch.json").exists());
}
