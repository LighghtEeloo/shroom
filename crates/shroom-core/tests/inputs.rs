use shroom_core::{Error, HostPort, WorkspaceName};

#[test]
fn workspace_name_boundaries() {
    for name in ["a", "0", "a-b-9", "trailing-", &"a".repeat(48)] {
        let parsed: WorkspaceName = name.parse().unwrap();
        assert_eq!(parsed.as_str(), name);
    }
    for name in [
        "",
        "-a",
        "A",
        "a_b",
        "a/b",
        "..",
        "a b",
        "a\n",
        "é",
        &"a".repeat(49),
    ] {
        assert!(
            matches!(name.parse::<WorkspaceName>(), Err(Error::InvalidName)),
            "{name:?}"
        );
    }
}

#[test]
fn host_port_boundaries() {
    for port in [1024, 2222, 65535] {
        assert_eq!(HostPort::try_from(port).unwrap().get(), port as u16);
    }
    for port in [0, 22, 1023, 65536, u32::MAX] {
        assert!(
            matches!(HostPort::try_from(port), Err(Error::InvalidPort(value)) if value == port)
        );
    }
}
