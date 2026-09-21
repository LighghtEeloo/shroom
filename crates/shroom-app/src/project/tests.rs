use super::*;

struct Fixture;

impl Fixture {
    fn script(script: &str) -> std::process::Command {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }
}

#[tokio::test]
async fn directory_checks_distinguish_rejection_transport_errors_and_timeouts() {
    let directory = "/mnt/project".parse().unwrap();
    let timeout = Duration::from_secs(2);
    ProjectCheck::run(Fixture::script("exit 0"), &directory, timeout)
        .await
        .unwrap();
    assert!(
        matches!(ProjectCheck::run(Fixture::script("exit 72"), &directory, timeout).await,
        Err(Error::DirectoryUnavailable(path)) if path == directory)
    );
    assert!(
        matches!(ProjectCheck::run(Fixture::script("echo 'connection refused' >&2; exit 255"), &directory, timeout).await,
        Err(Error::Ssh { diagnostic, .. }) if diagnostic.contains("connection refused"))
    );
    assert!(
        matches!(ProjectCheck::run(Fixture::script("while :; do :; done"), &directory, Duration::from_millis(30)).await,
        Err(Error::DirectoryCheckTimedOut(path)) if path == directory)
    );
}

#[tokio::test]
async fn checks_drain_large_output_without_growing_diagnostics() {
    let directory = "/mnt/project".parse().unwrap();
    let command = Fixture::script(
        "i=0; while [ $i -lt 10000 ]; do echo 'out'; echo 'diagnostic' >&2; i=$((i+1)); done; exit 255",
    );
    let Err(Error::Ssh { diagnostic, .. }) =
        ProjectCheck::run(command, &directory, Duration::from_secs(5)).await
    else {
        panic!("expected SSH failure");
    };
    assert!(diagnostic.len() < 4200);
}
