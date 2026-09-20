use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;
use crate::test_support::Fixture;

struct Data;

impl Data {
    fn attachment() -> Attachment {
        Codex::attachment(&"alpha".parse().unwrap(), Fixture::connection("alpha")).unwrap()
    }

    fn backups(directory: &Path) -> Vec<PathBuf> {
        fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("config.shroom-backup-")
            })
            .collect()
    }
}

#[tokio::test]
async fn registration_preserves_other_hosts_and_refreshes_the_same_entry() {
    let home = tempfile::tempdir().unwrap();
    let directory = home.path().join(".ssh");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("config");
    let original = "# personal config\r\nServerAliveInterval 45\r\nHost existing\r\n    HostName existing.example\r\n    User me\r\n    IdentityFile /tmp/personal-key\r\n";
    fs::write(&path, original).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let attachment = Data::attachment();
    let before = Codex::resolve(&path, "existing").await.unwrap();
    Codex::register(directory.clone(), attachment.clone())
        .await
        .unwrap();
    let first = fs::read_to_string(&path).unwrap();
    assert!(first.ends_with(original));
    assert_eq!(Codex::resolve(&path, "existing").await.unwrap(), before);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let backups = Data::backups(&directory);
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), original);
    assert_eq!(
        fs::metadata(&backups[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );

    Codex::register(directory.clone(), attachment.clone())
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), first);
    assert_eq!(Data::backups(&directory).len(), 1);

    let mut connection = attachment.connection().clone();
    connection.endpoint = "127.0.0.1:34567".parse().unwrap();
    let refreshed = Codex::attachment(&"alpha".parse().unwrap(), connection).unwrap();
    assert_eq!(refreshed.alias(), attachment.alias());
    Codex::register(directory.clone(), refreshed).await.unwrap();
    let updated = fs::read_to_string(&path).unwrap();
    assert_eq!(updated.matches("# BEGIN SHROOM CODEX").count(), 1);
    assert!(updated.contains("Port 34567\n"));
    assert!(updated.ends_with(original));
    assert_eq!(Data::backups(&directory).len(), 2);
}

#[tokio::test]
async fn new_config_is_private_and_distinct_workspaces_keep_their_own_entries() {
    let home = tempfile::tempdir().unwrap();
    let directory = home.path().join(".ssh");
    let attachment = Data::attachment();
    Codex::register(directory.clone(), attachment.clone())
        .await
        .unwrap();
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(directory.join("config"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(Data::backups(&directory).is_empty());
    let other = Codex::attachment(&"beta".parse().unwrap(), Fixture::connection("beta")).unwrap();
    assert_ne!(attachment.alias(), other.alias());
    Codex::register(directory.clone(), other.clone())
        .await
        .unwrap();
    let text = fs::read_to_string(directory.join("config")).unwrap();
    assert!(text.contains(&format!("Host {}\n", attachment.alias())));
    assert!(text.contains(&format!("Host {}\n", other.alias())));
    assert_eq!(text.matches("# BEGIN SHROOM CODEX").count(), 2);
    let longest = "a".repeat(48).parse().unwrap();
    assert!(Codex::attachment(&longest, Fixture::connection("alpha")).is_ok());
    let mut connection = Fixture::connection("alpha");
    // RFC 8032 test vector 2 supplies a distinct public host identity for the same display name.
    connection.host_key =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID1AF8PoQ4lakrcKp00bfrycmCzPLsSWjMDNVfEq9GYM"
            .parse()
            .unwrap();
    connection.host_key_alias = connection.host_key.alias();
    let separate_root = Codex::attachment(&"alpha".parse().unwrap(), connection).unwrap();
    assert_ne!(attachment.alias(), separate_root.alias());
    Codex::register(directory.clone(), separate_root)
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(directory.join("config"))
            .unwrap()
            .matches("# BEGIN SHROOM CODEX")
            .count(),
        3
    );
}

#[tokio::test]
async fn inherited_identities_commands_and_forwards_are_rejected_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    for option in [
        "IdentityFile /tmp/personal-key",
        "CertificateFile /tmp/personal-cert",
        "LocalForward 8080 localhost:8080",
        "RemoteForward 8081 localhost:8081",
        "DynamicForward 8082",
        "RemoteCommand echo unexpected",
        "KnownHostsCommand /usr/bin/true",
    ] {
        let original = format!("Host *\n    {option}\n");
        fs::write(&config, &original).unwrap();
        assert!(
            matches!(
                Codex::register(directory.path().to_owned(), Data::attachment()).await,
                Err(Error::ConflictingSshOptions)
            ),
            "accepted {option}"
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
        assert!(Data::backups(directory.path()).is_empty());
    }
    let original = "InvalidSshOption yes\n";
    fs::write(&config, original).unwrap();
    assert!(matches!(
        Codex::register(directory.path().to_owned(), Data::attachment()).await,
        Err(Error::InvalidSshConfig)
    ));
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
}

#[test]
fn conflicting_aliases_and_damaged_markers_preserve_the_original_config() {
    let attachment = Data::attachment();
    let alias = attachment.alias();
    for declaration in [
        format!("Host {alias}"),
        format!("hOsT=\"{alias}\" other"),
        format!("Host '{alias}'"),
    ] {
        assert!(matches!(
            PreparedConfig::merge(&declaration, &attachment),
            Err(Error::AliasConflict)
        ));
    }
    let managed = PreparedConfig::merge("", &attachment).unwrap();
    assert_eq!(
        PreparedConfig::merge(&managed, &attachment).unwrap(),
        managed
    );
    for damaged in [
        format!("{managed}{managed}"),
        managed.replace("# END SHROOM CODEX", "# edited end"),
        managed.trim_end().to_owned(),
        managed.replace("Host *\n", "Host unrelated\n"),
        managed.replace("    Port 2222\n", "Include /tmp/unrelated\n"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        fs::write(&config, &damaged).unwrap();
        assert!(matches!(
            PreparedConfig::new(directory.path(), &attachment),
            Err(Error::InvalidManagedEntry)
        ));
        assert_eq!(fs::read_to_string(config).unwrap(), damaged);
        assert!(Data::backups(directory.path()).is_empty());
    }
}

#[test]
fn concurrent_edits_and_locks_do_not_lose_user_changes() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let prepared = PreparedConfig::new(directory.path(), &Data::attachment()).unwrap();
    assert!(matches!(
        PreparedConfig::new(directory.path(), &Data::attachment()),
        Err(Error::ConfigBusy)
    ));
    let edited = "Host user-added\n    HostName user.example\n";
    fs::write(&config, edited).unwrap();
    assert!(matches!(prepared.commit(), Err(Error::ConfigChanged)));
    assert_eq!(fs::read_to_string(&config).unwrap(), edited);
    assert!(Data::backups(directory.path()).is_empty());
    let prepared = PreparedConfig::new(directory.path(), &Data::attachment()).unwrap();
    drop(prepared);
    assert!(PreparedConfig::new(directory.path(), &Data::attachment()).is_ok());
}

#[test]
fn symlinked_config_is_left_for_manual_setup() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("dotfiles-config");
    fs::write(&target, "Host personal\n").unwrap();
    symlink(&target, directory.path().join("config")).unwrap();
    assert!(matches!(
        PreparedConfig::new(directory.path(), &Data::attachment()),
        Err(Error::ConfigNotRegular)
    ));
    assert_eq!(fs::read_to_string(target).unwrap(), "Host personal\n");
    assert!(
        fs::symlink_metadata(directory.path().join("config"))
            .unwrap()
            .is_symlink()
    );
}
