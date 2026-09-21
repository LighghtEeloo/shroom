use super::*;
use crate::test_support::Fixture as Data;

struct Fixture;

impl AgentManager {
    pub(crate) fn preview(&self, name: &str, app: WebApp, tunnel: TunnelState) {
        self.updates.send_replace(vec![WebSession {
            id: 1,
            name: name.parse().unwrap(),
            app,
            connection: Data::connection(name),
            directory: "/home/developer/workspace".parse().unwrap(),
            launch: LaunchState::Running,
            tunnel,
            output: "Web app listening at http://127.0.0.1:5495/?token=preview\n".into(),
        }]);
    }
}

impl Fixture {
    fn context(role: ProcessRole) -> ProcessContext {
        ProcessContext {
            id: 1,
            role,
            updates: watch::channel(vec![WebSession {
                id: 1,
                name: "project".parse().unwrap(),
                app: WebApp::Kimi,
                connection: Data::connection("project"),
                directory: "/home/developer/workspace".parse().unwrap(),
                launch: LaunchState::Running,
                tunnel: TunnelState::Idle,
                output: String::new(),
            }])
            .0,
        }
    }

    fn script(text: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", text]);
        command
    }
}

#[test]
fn process_output_is_bounded_and_strips_terminal_control_sequences() {
    let mut output = OutputText::default();
    assert_eq!(output.decode(b"hello\x1b[3"), "hello");
    assert_eq!(
        output.decode(b"1m world\x1b[0m\n\x1b]8;;https://example.com\x07link\x1b]8;;\x1b\\"),
        " world\nlink"
    );
    let mut text = "é".repeat(OUTPUT_LIMIT);
    OutputText::append(&mut text, "end");
    assert!(text.len() <= OUTPUT_LIMIT);
    assert!(text.ends_with("end"));
}

#[tokio::test]
async fn failed_launch_retains_stderr_and_revokes_readiness() {
    let context = Fixture::context(ProcessRole::Launch);
    context.update(|session| session.tunnel = TunnelState::Ready("http://127.0.0.1:5494".into()));
    let (process, _) = ManagedProcess::spawn(
        Fixture::script("printf 'kimi: command not found\\n' >&2; exit 127"),
        context.clone(),
    )
    .unwrap();
    process.task.await.unwrap();
    let sessions = context.updates.borrow();
    assert_eq!(sessions[0].launch, LaunchState::Exited);
    assert!(sessions[0].output.contains("kimi: command not found"));
    assert!(matches!(sessions[0].tunnel, TunnelState::Idle));
}

#[tokio::test]
async fn tunnel_requires_remote_marker_and_ends_with_launch() {
    let context = Fixture::context(ProcessRole::Tunnel);
    let (process, ready) = ManagedProcess::spawn(
        Fixture::script("printf 'shroom-web-tunnel-ready\\n'; exec cat >/dev/null"),
        context.clone(),
    )
    .unwrap();
    time::timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    time::sleep(Duration::from_millis(50)).await;
    assert!(!process.task.is_finished());
    context.update(|session| session.launch = LaunchState::Exited);
    time::timeout(Duration::from_secs(2), process.task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        context.updates.borrow()[0].tunnel,
        TunnelState::Failed(_)
    ));

    let (process, ready) = ManagedProcess::spawn(
        Fixture::script("exit 1"),
        Fixture::context(ProcessRole::Tunnel),
    )
    .unwrap();
    assert!(
        time::timeout(Duration::from_secs(2), ready)
            .await
            .unwrap()
            .is_err()
    );
    process.close().await;
}

#[tokio::test]
async fn closing_a_process_reaps_it_and_old_updates_cannot_touch_replacement() {
    let context = Fixture::context(ProcessRole::Tunnel);
    let (process, ready) = ManagedProcess::spawn(
        Fixture::script("printf 'shroom-web-tunnel-ready\\n'; exec cat >/dev/null"),
        context.clone(),
    )
    .unwrap();
    ready.await.unwrap();
    time::timeout(Duration::from_secs(2), process.close())
        .await
        .unwrap();
    context.updates.send_modify(|sessions| sessions[0].id = 2);
    context.output("stale output");
    assert!(!context.updates.borrow()[0].output.contains("stale output"));
}

#[tokio::test]
async fn reconciliation_discards_only_changed_or_stopped_workspace_sessions() {
    let manager = AgentManager::default();
    let mut old = Fixture::context(ProcessRole::Launch)
        .updates
        .borrow()
        .clone();
    let mut other = old[0].clone();
    other.id = 2;
    other.name = "other".parse().unwrap();
    other.connection = Data::connection("other");
    old.push(other);
    manager.updates.send_replace(old);
    manager
        .reconcile(&[
            Data::workspace("project", shroom_core::SandboxStatus::Running),
            Data::workspace("other", shroom_core::SandboxStatus::Stopped),
        ])
        .await;
    assert_eq!(manager.updates.borrow().len(), 1);
    let mut changed = Data::workspace("project", shroom_core::SandboxStatus::Running);
    changed.preference = Ok(crate::preferences::WorkingDirectoryPreference::Custom(
        "/mnt/b".parse().unwrap(),
    ));
    manager.reconcile(&[changed.clone()]).await;
    assert_eq!(manager.updates.borrow().len(), 1);
    assert_eq!(
        manager.updates.borrow()[0].directory.as_str(),
        "/home/developer/workspace"
    );
    changed.preference = Err(std::sync::Arc::new(
        crate::preferences::Error::UnsupportedVersion(9),
    ));
    manager.reconcile(&[changed.clone()]).await;
    assert_eq!(manager.updates.borrow().len(), 1);
    changed
        .workspace
        .ssh
        .as_mut()
        .unwrap()
        .endpoint
        .set_port(3333);
    manager.reconcile(&[changed]).await;
    assert!(manager.updates.borrow().is_empty());
}

struct RuntimeFixture;

impl RuntimeFixture {
    async fn execute(
        backend: &crate::model::Backend,
        command: crate::model::Command,
    ) -> crate::model::Reply {
        Box::pin(backend.execute(command)).await
    }

    async fn wait_for_output(manager: &AgentManager, app: WebApp, needle: &str) {
        let mut updates = manager.subscribe();
        time::timeout(Duration::from_secs(10), async {
            loop {
                {
                    let sessions = updates.borrow_and_update();
                    if let Some(session) = sessions.iter().find(|session| session.app == app) {
                        if session.output.contains(needle) {
                            return;
                        }
                        assert_eq!(session.launch, LaunchState::Running, "{}", session.output);
                    }
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .expect("guest did not report its web URL");
    }

    async fn install(attachment: &Attachment) {
        use shroom_integrations::{RemoteCommand, Terminal};
        // Fixture executables exercise the real SSH recipe, guest cwd, port fallback, and HTTP
        // forwarding without installing commercial agents or making authenticated provider calls.
        let script = r#"mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/shroom-test-web" <<'PERL'
#!/usr/bin/perl
use IO::Socket::INET;
use Cwd qw(getcwd);
$| = 1;
my $kimi = $0 =~ /kimi$/;
my $port = $kimi ? 15497 : 15498;
my $requested = $kimi ? 5494 : 3080;
die "wrong launch arguments" unless join(' ', @ARGV) eq "web --no-open --host 127.0.0.1 --port $requested";
die "missing unbuffered output" if $kimi && $ENV{PYTHONUNBUFFERED} ne '1';
my $server = IO::Socket::INET->new(LocalAddr => '127.0.0.1', LocalPort => $port, Listen => 5, ReuseAddr => 1) or die $!;
print "directory=" . getcwd() . "\n";
print "http://127.0.0.1:$port/app?token=fixture#login\n";
while (my $client = $server->accept()) {
    while (my $line = <$client>) { last if $line eq "\r\n"; }
    print $client "HTTP/1.0 200 OK\r\nContent-Length: 9\r\n\r\nworkspace";
    close $client;
}
PERL
chmod +x "$HOME/.local/bin/shroom-test-web"
ln -sf shroom-test-web "$HOME/.local/bin/kimi"
ln -sf shroom-test-web "$HOME/.local/bin/dsh"
printf '%s\n' 'export PATH="$HOME/.local/bin:$PATH"' > "$HOME/.bash_profile"
"#;
        let remote = RemoteCommand::new("/bin/sh")
            .unwrap()
            .with_arg("-c")
            .unwrap()
            .with_arg(script)
            .unwrap();
        let output =
            tokio::process::Command::from(attachment.login_command(&remote, Terminal::None))
                .output()
                .await
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
#[ignore = "requires the pinned runtime, prepared image, virtualization, and free local ports"]
async fn managed_web_launch_forward_and_shutdown_with_a_real_guest() {
    use crate::model::{Backend, Command as Action, Outcome, WebForward};
    use shroom_core::Config;
    let directory = tempfile::Builder::new()
        .prefix("sw-")
        .tempdir_in("/tmp")
        .unwrap()
        .keep();
    eprintln!("web integration acceptance state: {}", directory.display());
    let backend = Backend::default();
    RuntimeFixture::execute(
        &backend,
        Action::Open {
            state_dir: directory.clone(),
            config: Config {
                runtime_executable: std::env::var_os("SHROOM_TEST_RUNTIME")
                    .expect("set SHROOM_TEST_RUNTIME")
                    .into(),
                firmware: std::env::var_os("SHROOM_TEST_FIRMWARE")
                    .expect("set SHROOM_TEST_FIRMWARE")
                    .into(),
            },
        },
    )
    .await
    .outcome
    .unwrap();
    RuntimeFixture::execute(
        &backend,
        Action::ImportImage(
            std::env::var_os("SHROOM_TEST_IMAGE_ARCHIVE")
                .expect("set SHROOM_TEST_IMAGE_ARCHIVE")
                .into(),
        ),
    )
    .await
    .outcome
    .unwrap();
    let port = std::env::var("SHROOM_WEB_TEST_PORT")
        .unwrap_or_else(|_| "34982".into())
        .parse::<u32>()
        .unwrap();
    assert!(port <= 65531);
    let name: WorkspaceName = "agents".parse().unwrap();
    let Outcome::Verified(connection) = RuntimeFixture::execute(
        &backend,
        Action::Create(name.clone(), port.try_into().unwrap(), Default::default()),
    )
    .await
    .outcome
    .unwrap() else {
        panic!("expected verified connection")
    };
    let attachment = Attachment::new("shroom-agents".parse().unwrap(), *connection).unwrap();
    RuntimeFixture::install(&attachment).await;

    for (index, app) in [WebApp::Kimi, WebApp::DeepSeekHarness]
        .into_iter()
        .enumerate()
    {
        RuntimeFixture::execute(
            &backend,
            Action::LaunchWeb(name.clone(), app, app.default_port()),
        )
        .await
        .outcome
        .unwrap();
        RuntimeFixture::wait_for_output(&backend.agents, app, "token=fixture").await;
        let duplicate = RuntimeFixture::execute(
            &backend,
            Action::LaunchWeb(name.clone(), app, app.default_port()),
        )
        .await;
        assert!(matches!(
            duplicate.outcome,
            Err(crate::model::Error::Agent(Error::AlreadyRunning))
        ));
        let expected = if index == 0 {
            "/home/developer/workspace"
        } else {
            "/home/developer"
        };
        RuntimeFixture::wait_for_output(&backend.agents, app, &format!("directory={expected}\n"))
            .await;
        if index == 0 {
            let before = backend.agents.subscribe().borrow()[0].clone();
            RuntimeFixture::execute(
                &backend,
                Action::SetWorkingDirectory(name.clone(), "/home/developer".parse().unwrap()),
            )
            .await
            .outcome
            .unwrap();
            RuntimeFixture::execute(&backend, Action::Refresh)
                .await
                .outcome
                .unwrap();
            let after = backend.agents.subscribe().borrow()[0].clone();
            assert_eq!(before.id, after.id);
            assert_eq!(before.directory, after.directory);
            assert!(after.output.contains("token=fixture"));
        }
        let reported = format!("http://127.0.0.1:{}/app?token=fixture#login", 15497 + index);
        let local = (port + 1 + index as u32).try_into().unwrap();
        let forward = WebForward {
            guest: reported.parse().unwrap(),
            local_port: local,
        };
        RuntimeFixture::execute(
            &backend,
            Action::ForwardWeb(name.clone(), app, forward.clone()),
        )
        .await
        .outcome
        .unwrap();
        let url = {
            let updates = backend.agents.subscribe();
            let snapshots = updates.borrow();
            let session = snapshots.iter().find(|session| session.app == app).unwrap();
            let TunnelState::Ready(url) = &session.tunnel else {
                panic!("{}", session.output)
            };
            assert_eq!(
                url,
                &format!("http://127.0.0.1:{local}/app?token=fixture#login")
            );
            url.clone()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        assert_eq!(
            client.get(&url).send().await.unwrap().text().await.unwrap(),
            "workspace"
        );

        if app == WebApp::Kimi {
            let occupied = tokio::net::TcpListener::bind(("127.0.0.1", (port + 3) as u16))
                .await
                .unwrap();
            let conflict = WebForward {
                local_port: (port + 3).try_into().unwrap(),
                ..forward.clone()
            };
            let reply =
                RuntimeFixture::execute(&backend, Action::ForwardWeb(name.clone(), app, conflict))
                    .await;
            assert!(matches!(
                reply.outcome,
                Err(crate::model::Error::Agent(Error::TunnelUnavailable))
            ));
            assert!(matches!(
                backend
                    .agents
                    .subscribe()
                    .borrow()
                    .iter()
                    .find(|session| session.app == app)
                    .unwrap()
                    .tunnel,
                TunnelState::Failed(_)
            ));
            drop(occupied);
            RuntimeFixture::execute(&backend, Action::ForwardWeb(name.clone(), app, forward))
                .await
                .outcome
                .unwrap();
            RuntimeFixture::execute(&backend, Action::CloseWeb(name.clone(), app))
                .await
                .outcome
                .unwrap();
            assert!(
                tokio::net::TcpStream::connect(("127.0.0.1", local.get()))
                    .await
                    .is_err()
            );
        }
    }
    RuntimeFixture::execute(&backend, Action::Stop(name.clone()))
        .await
        .outcome
        .unwrap();
    assert!(backend.agents.subscribe().borrow().is_empty());
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", (port + 2) as u16))
            .await
            .is_err()
    );
    let stopped_launch = RuntimeFixture::execute(
        &backend,
        Action::LaunchWeb(name.clone(), WebApp::Kimi, WebApp::Kimi.default_port()),
    )
    .await;
    assert!(matches!(
        stopped_launch.outcome,
        Err(crate::model::Error::SshUnavailable)
    ));
    RuntimeFixture::execute(&backend, Action::Start(name.clone()))
        .await
        .outcome
        .unwrap();
    RuntimeFixture::execute(
        &backend,
        Action::LaunchWeb(name.clone(), WebApp::Kimi, WebApp::Kimi.default_port()),
    )
    .await
    .outcome
    .unwrap();
    RuntimeFixture::wait_for_output(&backend.agents, WebApp::Kimi, "token=fixture").await;
    RuntimeFixture::execute(&backend, Action::Disconnect)
        .await
        .outcome
        .unwrap();
    assert!(backend.agents.subscribe().borrow().is_empty());
    // Reopening discovers the still-running workspace after the app-owned connection is gone.
    RuntimeFixture::execute(
        &backend,
        Action::Open {
            state_dir: directory.clone(),
            config: Config {
                runtime_executable: std::env::var_os("SHROOM_TEST_RUNTIME").unwrap().into(),
                firmware: std::env::var_os("SHROOM_TEST_FIRMWARE").unwrap().into(),
            },
        },
    )
    .await
    .outcome
    .unwrap();
    RuntimeFixture::execute(&backend, Action::Stop(name.clone()))
        .await
        .outcome
        .unwrap();
    RuntimeFixture::execute(&backend, Action::Remove(name))
        .await
        .outcome
        .unwrap();
    RuntimeFixture::execute(&backend, Action::Disconnect)
        .await
        .outcome
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
