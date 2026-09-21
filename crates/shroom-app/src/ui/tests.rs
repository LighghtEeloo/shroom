use std::time::{Duration, Instant};

use freya_testing::{TestingNode, TestingRunner};

use super::*;
use crate::model::AccessCheck;
use crate::test_support::Fixture as Data;

struct Fixture;

#[test]
fn stopped_workspace_directory_editor_preserves_invalid_input_and_cancel_discards_it() {
    let mut runner = Fixture::runner(Fixture::workspaces(SandboxStatus::Stopped), (1320., 1500.));
    Fixture::click(&mut runner, "Edit working directory");
    let center = Fixture::paragraph(&runner, "/home/developer/workspace")
        .unwrap()
        .layout()
        .area
        .center();
    runner.click_cursor((f64::from(center.x), f64::from(center.y)));
    for _ in 0..40 {
        runner.press_key(Key::Named(NamedKey::ArrowRight));
    }
    runner.write_text(" ");
    Fixture::click(&mut runner, "Save working directory");
    assert!(
        Fixture::paragraph(
            &runner,
            &shroom_integrations::Error::InvalidGuestDirectory.to_string()
        )
        .is_some()
    );
    assert!(Fixture::label(&runner, "Guest folder").is_some());
    assert!(Fixture::label(&runner, "Saving default working directory…").is_none());
    assert!(Fixture::label(&runner, "Starting workspace…").is_none());
    Fixture::click(&mut runner, "Cancel directory edit");
    Fixture::click(&mut runner, "Edit working directory");
    assert!(Fixture::paragraph(&runner, "/home/developer/workspace").is_some());
}

#[test]
fn corrupt_preferences_keep_home_login_and_existing_web_actions_available() {
    let mut initial = Fixture::verified();
    initial.workspaces[0].preference =
        Err(Arc::new(crate::preferences::Error::UnsupportedVersion(9)));
    let app = Shroom {
        initial,
        ..Shroom::default()
    };
    app.backend.agents.preview(
        "project",
        shroom_integrations::WebApp::Kimi,
        crate::agents::TunnelState::Ready("http://127.0.0.1:5494/".into()),
    );
    let mut runner = Fixture::with_app(app, (1320., 1500.));
    assert!(!Fixture::button_disabled(&runner, "Copy home login"));
    assert!(Fixture::button_disabled(&runner, "Copy terminal command"));
    Fixture::click(&mut runner, "Agents");
    Fixture::click(&mut runner, "Kimi");
    assert!(Fixture::label(&runner, "Session directory").is_some());
    assert!(!Fixture::button_disabled(&runner, "Open in browser"));
}

impl Fixture {
    fn contains_label(node: &TestingNode, text: &str) -> bool {
        Label::try_downcast(node.element().as_ref()).is_some_and(|label| label.text == text)
            || node
                .children()
                .iter()
                .any(|child| Self::contains_label(child, text))
    }

    fn button_disabled(runner: &TestingRunner, name: &str) -> bool {
        let node = runner
            .find(|node, element| {
                Rect::try_downcast(element)
                    .filter(|rect| {
                        rect.accessibility.builder.role() == AccessibilityRole::Button
                            && Self::contains_label(&node, name)
                    })
                    .map(|_| node)
            })
            .expect("named button");
        !Rect::try_downcast(node.element().as_ref())
            .unwrap()
            .accessibility
            .a11y_focusable
            .is_enabled()
    }

    fn runner(initial: Model, size: (f32, f32)) -> TestingRunner {
        Self::with_app(
            Shroom {
                initial,
                ..Shroom::default()
            },
            size,
        )
    }

    fn with_app(app: Shroom, size: (f32, f32)) -> TestingRunner {
        let mut runner =
            TestingRunner::new(move || app.render().into_element(), size.into(), |_| {}, 1.).0;
        runner.sync_and_update();
        runner
    }

    fn settle(runner: &mut TestingRunner, ready: impl Fn(&TestingRunner) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            runner.poll_n(Duration::from_millis(10), 1);
            if ready(runner) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "UI did not receive the operation result"
            );
        }
    }

    fn paragraph(runner: &TestingRunner, text: &str) -> Option<TestingNode> {
        runner.find(|node, element| {
            Paragraph::try_downcast(element)
                .filter(|paragraph| paragraph.spans.iter().any(|span| span.text == text))
                .map(|_| node)
        })
    }

    fn label(runner: &TestingRunner, text: &str) -> Option<TestingNode> {
        runner.find(|node, element| {
            Label::try_downcast(element)
                .filter(|label| label.text == text)
                .map(|_| node)
        })
    }

    fn control(runner: &TestingRunner, name: &str) -> Option<TestingNode> {
        runner.find(|node, element| {
            Rect::try_downcast(element)
                .filter(|rect| {
                    rect.accessibility.builder.role() == AccessibilityRole::Button
                        && rect.accessibility.builder.label() == Some(name)
                })
                .map(|_| node)
        })
    }

    fn click(runner: &mut TestingRunner, text: &str) {
        let node = Self::control(runner, text)
            .or_else(|| Self::label(runner, text))
            .unwrap_or_else(|| panic!("missing control: {text}"));
        assert!(node.is_visible(), "label is outside the viewport: {text}");
        let center = node.layout().area.center();
        runner.click_cursor((f64::from(center.x), f64::from(center.y)));
    }

    fn screenshot(runner: &mut TestingRunner, path: String) {
        // SVGs inherit their color after layout; capture the subsequent painted frame.
        runner.poll_n(Duration::from_millis(10), 2);
        runner.render_to_file(path);
    }

    fn workspaces(state: SandboxStatus) -> Model {
        Model {
            session: Data::session(),
            catalog_current: true,
            image: ImageStatus::Ready,
            selected: Some("project".parse().unwrap()),
            workspaces: vec![Data::workspace("project", state)],
            ..Model::default()
        }
    }

    fn verified() -> Model {
        Model {
            checks: vec![AccessCheck {
                name: "project".parse().unwrap(),
                at: std::time::UNIX_EPOCH + Duration::from_secs(36000),
                result: CheckResult::Verified(Box::new(Data::connection("project"))),
            }],
            ..Self::workspaces(SandboxStatus::Running)
        }
    }

    fn themed(
        initial: Model,
        size: (f32, f32),
        theme: PreferredTheme,
    ) -> (TestingRunner, State<PreferredTheme>) {
        let app = Shroom {
            initial,
            ..Shroom::default()
        };
        let (mut runner, mut preference) = TestingRunner::new(
            move || app.render().into_element(),
            size.into(),
            |runner| runner.run_in(|| Platform::get().preferred_theme),
            1.,
        );
        preference.set(theme);
        runner.poll_n(Duration::from_millis(10), 2);
        (runner, preference)
    }
}

#[test]
fn setup_and_empty_catalog_render_at_minimum_size() {
    let setup = Fixture::runner(Model::default(), (820., 640.));
    assert!(
        Fixture::label(&setup, "Open your workspace directory")
            .unwrap()
            .is_visible()
    );
    assert!(Fixture::label(&setup, "Runtime executable").is_some());
    let empty = Fixture::runner(
        Model {
            session: Data::session(),
            catalog_current: true,
            image: ImageStatus::Ready,
            ..Model::default()
        },
        (820., 640.),
    );
    assert!(
        Fixture::label(&empty, "Your next workspace starts here.")
            .unwrap()
            .is_visible()
    );
    assert!(
        Fixture::label(&empty, "New workspace")
            .unwrap()
            .is_visible()
    );
}

#[test]
fn connection_tabs_details_and_compact_navigation_remain_accessible() {
    let mut runner = Fixture::runner(Fixture::verified(), (820., 640.));
    assert!(Fixture::label(&runner, "Local workspaces").is_none());
    assert!(
        Fixture::label(&runner, "Ready to connect")
            .unwrap()
            .is_visible()
    );
    assert!(Fixture::label(&runner, "SSH verified · 10:00:00 UTC").is_some());
    assert!(
        Fixture::label(&runner, "Copy terminal command")
            .unwrap()
            .is_visible()
    );
    Fixture::click(&mut runner, "SSH config");
    assert!(
        Fixture::label(&runner, "Copy SSH config")
            .unwrap()
            .is_visible()
    );
    Fixture::click(&mut runner, "Toggle workspace details");
    assert!(Fixture::label(&runner, "Runtime storage").is_none());
    Fixture::click(&mut runner, "Show workspaces");
    assert!(
        Fixture::label(&runner, "Local workspaces")
            .unwrap()
            .is_visible()
    );
    Fixture::click(&mut runner, "New workspace");
    assert!(
        Fixture::label(&runner, "Workspace name")
            .unwrap()
            .is_visible()
    );
    assert!(Fixture::label(&runner, "Local workspaces").is_none());
    Fixture::click(&mut runner, "Cancel");
    assert!(Fixture::label(&runner, "Ready to connect").is_some());
}

#[test]
fn agent_picker_exposes_each_native_route_and_both_web_launchers() {
    let mut runner = Fixture::runner(Fixture::verified(), (1120., 1080.));
    Fixture::click(&mut runner, "Agents");
    for app in [
        shroom_integrations::NativeApp::Codex,
        shroom_integrations::NativeApp::ClaudeDesktop,
        shroom_integrations::NativeApp::Cursor,
        shroom_integrations::NativeApp::ZCode,
    ] {
        Fixture::click(&mut runner, app.name());
        assert!(Fixture::label(&runner, &format!("Connect with {}", app.name())).is_some());
        if app == shroom_integrations::NativeApp::Codex {
            assert!(
                Fixture::label(&runner, "Add to Codex")
                    .unwrap()
                    .is_visible()
            );
            assert!(Fixture::label(&runner, "Copy SSH config").is_none());
        } else {
            assert!(Fixture::label(&runner, app.setup()).is_some());
            assert!(Fixture::label(&runner, "Add to Codex").is_none());
        }
    }
    assert!(
        Fixture::label(
            &runner,
            "Host-key verification in ZCode is not yet validated."
        )
        .is_some()
    );
    assert!(Fixture::label(&runner, "Copy connection details").is_some());
    for app in [
        shroom_integrations::WebApp::Kimi,
        shroom_integrations::WebApp::DeepSeekHarness,
    ] {
        Fixture::click(&mut runner, app.name());
        assert!(
            Fixture::label(&runner, &format!("Launch {}", app.name()))
                .unwrap()
                .is_visible()
        );
        assert!(Fixture::paragraph(&runner, &app.default_port().to_string()).is_some());
        assert!(Fixture::label(&runner, "Open in browser").is_none());
    }
    Fixture::click(&mut runner, "Command");
    assert!(Fixture::label(&runner, "Copy terminal command").is_some());
}

#[test]
fn invalid_web_launch_input_is_retained_without_dispatching() {
    let mut runner = Fixture::runner(Fixture::verified(), (1120., 1080.));
    Fixture::click(&mut runner, "Agents");
    Fixture::click(&mut runner, "Kimi");
    let center = Fixture::paragraph(&runner, "5494")
        .unwrap()
        .layout()
        .area
        .center();
    runner.click_cursor((f64::from(center.x), f64::from(center.y)));
    runner.write_text("oops");
    Fixture::click(&mut runner, "Launch Kimi");
    assert!(
        Fixture::paragraph(&runner, &crate::model::Error::InvalidWebPort.to_string()).is_some()
    );
    assert!(Fixture::label(&runner, "Launching web app through SSH…").is_none());
    assert!(
        runner
            .find(|node, element| Paragraph::try_downcast(element)
                .filter(|paragraph| paragraph
                    .spans
                    .iter()
                    .any(|span| span.text.contains("oops")))
                .map(|_| node))
            .is_some()
    );
}

#[test]
fn web_browser_actions_require_a_checked_current_connection() {
    use crate::agents::TunnelState;
    use shroom_integrations::WebApp;
    for (tunnel, pending, ready) in [
        (TunnelState::Idle, false, false),
        (TunnelState::Connecting, false, false),
        (TunnelState::Failed("Port is occupied".into()), false, false),
        (
            TunnelState::Ready("http://127.0.0.1:5494/?token=preview".into()),
            false,
            true,
        ),
        (
            TunnelState::Ready("http://127.0.0.1:5494/?token=preview".into()),
            true,
            false,
        ),
    ] {
        let app = Shroom {
            initial: Model {
                pending: pending.then_some(Command::Refresh),
                ..Fixture::verified()
            },
            ..Shroom::default()
        };
        app.backend.agents.preview("project", WebApp::Kimi, tunnel);
        let mut runner = Fixture::with_app(app, (1120., 1080.));
        Fixture::click(&mut runner, "Agents");
        Fixture::click(&mut runner, "Kimi");
        assert_eq!(Fixture::label(&runner, "Open in browser").is_some(), ready);
        assert_eq!(Fixture::label(&runner, "Copy browser URL").is_some(), ready);
    }
    let mut initial = Fixture::verified();
    initial.workspaces[0]
        .workspace
        .ssh
        .as_mut()
        .unwrap()
        .endpoint
        .set_port(3333);
    let app = Shroom {
        initial,
        ..Shroom::default()
    };
    app.backend.agents.preview(
        "project",
        WebApp::Kimi,
        TunnelState::Ready("http://127.0.0.1:5494/?token=preview".into()),
    );
    let mut runner = Fixture::with_app(app, (1120., 1080.));
    Fixture::click(&mut runner, "Agents");
    Fixture::click(&mut runner, "Kimi");
    assert!(Fixture::label(&runner, "Open in browser").is_none());
}

#[test]
fn web_process_updates_reach_the_ui_after_navigation() {
    let app = Shroom {
        initial: Fixture::verified(),
        ..Shroom::default()
    };
    let backend = app.backend.clone();
    let mut runner = Fixture::with_app(app, (1120., 1080.));
    Fixture::click(&mut runner, "Agents");
    Fixture::click(&mut runner, "Kimi");
    backend.agents.preview(
        "project",
        shroom_integrations::WebApp::Kimi,
        crate::agents::TunnelState::Ready("http://127.0.0.1:5494/".into()),
    );
    Fixture::settle(&mut runner, |runner| {
        Fixture::label(runner, "Open in browser").is_some()
    });
    backend.agents.preview(
        "project",
        shroom_integrations::WebApp::Kimi,
        crate::agents::TunnelState::Failed("Tunnel closed".into()),
    );
    Fixture::settle(&mut runner, |runner| {
        Fixture::label(runner, "Tunnel closed").is_some()
    });
    assert!(Fixture::label(&runner, "Open in browser").is_none());
}

#[test]
fn icon_controls_support_keyboard_activation_and_named_tooltips() {
    let mut runner = Fixture::runner(Fixture::verified(), (820., 640.));
    // The first compact toolbar control is reachable through normal keyboard navigation.
    runner.press_key(Key::Named(NamedKey::Tab));
    runner.poll_n(Duration::from_millis(10), 2);
    runner.press_key(Key::Named(NamedKey::Enter));
    assert!(Fixture::label(&runner, "Local workspaces").is_some());
    Fixture::click(&mut runner, "Hide workspaces");

    let details = Fixture::control(&runner, "Toggle workspace details").unwrap();
    assert_eq!(
        Rect::try_downcast(details.element().as_ref())
            .unwrap()
            .accessibility
            .builder
            .is_expanded(),
        Some(true)
    );
    Fixture::click(&mut runner, "Toggle workspace details");
    assert!(Fixture::label(&runner, "Runtime storage").is_none());
    runner.poll_n(Duration::from_millis(10), 2);
    runner.press_key(Key::Named(NamedKey::Enter));
    assert!(Fixture::label(&runner, "Runtime storage").is_some());
    runner.press_key(Key::Character(" ".into()));
    assert!(Fixture::label(&runner, "Runtime storage").is_none());

    let center = Fixture::control(&runner, "Refresh workspaces")
        .unwrap()
        .layout()
        .area
        .center();
    runner.move_cursor((f64::from(center.x), f64::from(center.y)));
    runner.poll(Duration::from_millis(20), Duration::from_millis(800));
    assert!(
        Fixture::label(&runner, "Refresh workspaces")
            .unwrap()
            .is_visible()
    );
}

#[test]
fn pending_operations_expose_disabled_icon_actions() {
    let initial = Model {
        pending: Some(Command::Refresh),
        ..Fixture::verified()
    };
    let mut runner = Fixture::runner(initial, (1120., 800.));
    for name in ["Refresh workspaces", "Workspace actions"] {
        let node = Fixture::control(&runner, name).unwrap();
        assert!(
            Rect::try_downcast(node.element().as_ref())
                .unwrap()
                .accessibility
                .builder
                .is_disabled()
        );
        Fixture::click(&mut runner, name);
    }
    assert!(Fixture::label(&runner, "Stop workspace").is_none());
    assert!(Fixture::label(&runner, "Refreshing workspaces…").is_some());
    Fixture::click(&mut runner, "Agents");
    assert!(Fixture::label(&runner, "Add to Codex").is_some());
    Fixture::click(&mut runner, "Add to Codex");
    assert!(Fixture::label(&runner, "Registering SSH connection and opening Codex…").is_none());
    assert!(Fixture::label(&runner, "Refreshing workspaces…").is_some());
    // Inspection remains available while the worker is busy.
    Fixture::click(&mut runner, "Toggle workspace details");
    assert!(Fixture::label(&runner, "Runtime storage").is_none());
}

#[test]
fn creation_preserves_invalid_input_and_does_not_dispatch_it() {
    let mut runner = Fixture::runner(Fixture::workspaces(SandboxStatus::Stopped), (1120., 800.));
    Fixture::click(&mut runner, "New workspace");
    let center = Fixture::paragraph(&runner, "my-project")
        .unwrap()
        .layout()
        .area
        .center();
    runner.click_cursor((f64::from(center.x), f64::from(center.y)));
    runner.write_text("Invalid Name");
    Fixture::click(&mut runner, "Create workspace");
    assert!(Fixture::paragraph(&runner, "Invalid Name").is_some());
    assert!(Fixture::paragraph(&runner, &shroom_core::Error::InvalidName.to_string()).is_some());
    assert!(Fixture::label(&runner, "Creating workspace and verifying SSH…").is_none());
    Fixture::click(&mut runner, "Cancel");
    Fixture::click(&mut runner, "New workspace");
    assert!(Fixture::paragraph(&runner, "Invalid Name").is_some());
}

#[test]
fn failed_creation_opens_recovery_only_when_the_requested_artifacts_exist() {
    let name: WorkspaceName = "partial".parse().unwrap();
    let command = Command::Create(name.clone(), 2222.try_into().unwrap(), Default::default());
    for (workspaces, incomplete, recovery) in [
        (
            vec![Data::workspace("partial", SandboxStatus::Running)],
            vec![],
            true,
        ),
        (vec![], vec![name.clone()], true),
        (
            vec![Data::workspace("unrelated", SandboxStatus::Stopped)],
            vec![],
            false,
        ),
        (vec![], vec![], false),
    ] {
        let mut model = Model::default();
        model.begin(&command);
        model.apply(crate::model::Reply {
            session: Data::session(),
            selection: Some(name.clone()),
            outcome: Err(crate::model::Error::SshUnavailable),
            catalog: Ok(crate::model::Catalog {
                workspaces,
                incomplete,
            }),
            image: Ok(ImageStatus::Ready),
        });
        let mut nav = Navigation {
            page: Page::Create,
            ..Navigation::default()
        };
        nav.reconcile(&command, &model, false);
        assert_eq!(nav.page == Page::Workspace, recovery);
        assert!(!model.errors.is_empty());
    }
}

#[test]
fn system_appearance_change_preserves_selection_and_form_navigation() {
    let (mut runner, mut preference) =
        Fixture::themed(Fixture::verified(), (1120., 800.), PreferredTheme::Light);
    Fixture::click(&mut runner, "SSH config");
    preference.set(PreferredTheme::Dark);
    runner.poll_n(Duration::from_millis(10), 2);
    assert!(
        Fixture::label(&runner, "Copy SSH config")
            .unwrap()
            .is_visible()
    );
    assert!(
        Fixture::label(&runner, "Ready to connect")
            .unwrap()
            .is_visible()
    );
    Fixture::click(&mut runner, "Inspect with msb");
    let command = Data::session().unwrap().inspection_command();
    assert!(Fixture::paragraph(&runner, &command).is_some());
}

#[test]
fn remove_requires_confirmation_and_cancel_preserves_the_workspace() {
    let mut runner = Fixture::runner(Fixture::workspaces(SandboxStatus::Stopped), (1040., 900.));
    assert!(Fixture::label(&runner, "Delete workspace").is_none());
    Fixture::click(&mut runner, "Workspace actions");
    Fixture::click(&mut runner, "Delete workspace");
    assert!(
        Fixture::label(
            &runner,
            "Permanently remove project and its private guest files?"
        )
        .is_some()
    );
    Fixture::click(&mut runner, "Cancel");
    assert!(Fixture::label(&runner, "Delete workspace").is_none());
    assert!(Fixture::label(&runner, "Workspaces · 1").is_some());
}

#[test]
fn running_and_busy_workspaces_cannot_be_removed() {
    for initial in [
        Fixture::workspaces(SandboxStatus::Running),
        Model {
            pending: Some(Command::Refresh),
            ..Fixture::workspaces(SandboxStatus::Stopped)
        },
    ] {
        let mut runner = Fixture::runner(initial, (1040., 900.));
        Fixture::click(&mut runner, "Workspace actions");
        assert!(Fixture::label(&runner, "Delete workspace").is_none());
        assert!(Fixture::label(&runner, "Workspaces · 1").is_some());
    }
}

#[test]
fn missing_image_disables_creation_and_incomplete_setup_requires_confirmation() {
    let mut runner = Fixture::runner(
        Model {
            session: Data::session(),
            catalog_current: true,
            image: ImageStatus::Missing,
            incomplete: vec!["zydeco".parse().unwrap()],
            selected: Some("zydeco".parse().unwrap()),
            ..Model::default()
        },
        (1040., 1100.),
    );
    Fixture::click(&mut runner, "New workspace");
    assert!(
        Fixture::label(&runner, "Prepare your workspace image")
            .unwrap()
            .is_visible()
    );
    Fixture::click(&mut runner, "Create workspace");
    assert!(Fixture::label(&runner, "Creating workspace and verifying SSH…").is_none());
    Fixture::click(&mut runner, "Open environment");
    assert!(Fixture::label(&runner, "Import image").is_some());
    Fixture::click(&mut runner, "zydeco");
    assert!(Fixture::label(&runner, "Confirm cleanup").is_none());
    Fixture::click(&mut runner, "Remove incomplete setup");
    assert!(Fixture::label(&runner, "Remove the incomplete setup for zydeco?").is_some());
    Fixture::click(&mut runner, "Cancel");
    assert!(Fixture::label(&runner, "Confirm cleanup").is_none());
    assert!(Fixture::label(&runner, "Clean up zydeco").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmed_operations_deliver_errors_after_the_confirmation_unmounts() {
    for (initial, action, confirmation, progress) in [
        (
            Model {
                session: Data::session(),
                catalog_current: true,
                image: ImageStatus::Ready,
                incomplete: vec!["zydeco".parse().unwrap()],
                selected: Some("zydeco".parse().unwrap()),
                ..Model::default()
            },
            "Remove incomplete setup",
            "Confirm cleanup",
            "Removing incomplete setup…",
        ),
        (
            Fixture::workspaces(SandboxStatus::Stopped),
            "Workspace actions",
            "Delete workspace",
            "Removing workspace…",
        ),
    ] {
        // The disconnected worker must report its error even after the originating button disappears.
        let mut runner = Fixture::runner(initial, (1040., 1100.));
        Fixture::click(&mut runner, action);
        if action == "Workspace actions" {
            Fixture::click(&mut runner, "Delete workspace");
        }
        Fixture::click(&mut runner, confirmation);
        assert!(Fixture::label(&runner, confirmation).is_none());
        Fixture::settle(&mut runner, |runner| {
            Fixture::label(runner, "Open workspaces").is_some()
        });
        assert!(Fixture::label(&runner, progress).is_none());
        assert!(Fixture::paragraph(&runner, "Open a workspace directory first").is_some());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the pinned runtime and firmware; does not boot a VM"]
async fn confirmed_cleanup_completes_through_the_ui_with_a_real_core() {
    let directory = tempfile::Builder::new()
        .prefix("sh-ui-")
        .tempdir_in("/tmp")
        .unwrap();
    let backend = Arc::new(Backend::default());
    backend
        .execute(Command::Open {
            state_dir: directory.path().into(),
            config: shroom_core::Config {
                runtime_executable: std::env::var_os("SHROOM_TEST_RUNTIME")
                    .expect("set SHROOM_TEST_RUNTIME")
                    .into(),
                firmware: std::env::var_os("SHROOM_TEST_FIRMWARE")
                    .expect("set SHROOM_TEST_FIRMWARE")
                    .into(),
            },
        })
        .await
        .outcome
        .unwrap();
    let incomplete = directory.path().join("access/zydeco");
    std::fs::create_dir(&incomplete).unwrap();
    std::fs::write(incomplete.join("fixture"), b"incomplete setup").unwrap();
    let mut initial = Model::default();
    initial.apply(backend.execute(Command::Refresh).await);
    let mut runner = Fixture::with_app(
        Shroom {
            backend: backend.clone(),
            initial,
        },
        (1040., 1100.),
    );

    Fixture::click(&mut runner, "Remove incomplete setup");
    Fixture::click(&mut runner, "Confirm cleanup");
    Fixture::settle(&mut runner, |runner| {
        Fixture::label(runner, "Removing incomplete setup…").is_none()
            && Fixture::label(runner, "Clean up zydeco").is_none()
    });
    assert!(!incomplete.exists());
    assert!(Fixture::label(&runner, "Workspaces · 0").is_some());

    Fixture::click(&mut runner, "Refresh workspaces");
    Fixture::settle(&mut runner, |runner| {
        Fixture::label(runner, "Refreshing workspaces…").is_none()
    });
    Fixture::click(&mut runner, "Local workspaces");
    Fixture::click(&mut runner, "Change directory");
    Fixture::settle(&mut runner, |runner| {
        Fixture::label(runner, "Open workspaces").is_some()
    });
    let lock = std::fs::File::open(directory.path().join("core.lock")).unwrap();
    lock.try_lock().unwrap();
}

#[test]
#[ignore = "writes screenshots for manual visual review"]
fn render_preview() {
    let directory = std::env::var("SHROOM_PREVIEW_DIR").expect("set SHROOM_PREVIEW_DIR");
    let mut setup = Fixture::runner(Model::default(), (820., 640.));
    Fixture::screenshot(&mut setup, format!("{directory}/setup.png"));
    let mut workspaces =
        Fixture::runner(Fixture::workspaces(SandboxStatus::Stopped), (1120., 800.));
    Fixture::screenshot(&mut workspaces, format!("{directory}/workspaces.png"));
    Fixture::click(&mut workspaces, "Edit working directory");
    Fixture::screenshot(
        &mut workspaces,
        format!("{directory}/working-directory.png"),
    );
    let mut recovery = Fixture::runner(
        Model {
            session: Data::session(),
            catalog_current: true,
            image: ImageStatus::Missing,
            incomplete: vec!["zydeco".parse().unwrap()],
            selected: Some("zydeco".parse().unwrap()),
            ..Model::default()
        },
        (1120., 800.),
    );
    Fixture::screenshot(&mut recovery, format!("{directory}/recovery.png"));
    for (theme, filename) in [
        (PreferredTheme::Light, "running-light"),
        (PreferredTheme::Dark, "running-dark"),
    ] {
        let mut initial = Fixture::verified();
        initial
            .workspaces
            .push(Data::workspace("research", SandboxStatus::Stopped));
        initial.incomplete.push("zydeco".parse().unwrap());
        let (mut runner, _) = Fixture::themed(initial, (1120., 800.), theme);
        Fixture::screenshot(&mut runner, format!("{directory}/{filename}.png"));
    }
    let mut compact = Fixture::runner(Fixture::verified(), (820., 640.));
    Fixture::screenshot(&mut compact, format!("{directory}/compact.png"));
    Fixture::click(&mut compact, "Show workspaces");
    Fixture::screenshot(&mut compact, format!("{directory}/compact-navigation.png"));
    let mut new_workspace =
        Fixture::runner(Fixture::workspaces(SandboxStatus::Stopped), (1120., 800.));
    Fixture::click(&mut new_workspace, "New workspace");
    Fixture::screenshot(&mut new_workspace, format!("{directory}/create.png"));
    Fixture::click(&mut recovery, "Local workspaces");
    Fixture::screenshot(&mut recovery, format!("{directory}/environment.png"));
    for (theme, filename) in [
        (PreferredTheme::Light, "agents-light"),
        (PreferredTheme::Dark, "agents-dark"),
    ] {
        let (mut runner, _) = Fixture::themed(Fixture::verified(), (1120., 800.), theme);
        Fixture::click(&mut runner, "Agents");
        Fixture::screenshot(&mut runner, format!("{directory}/{filename}.png"));
    }
    let mut agents = Fixture::runner(Fixture::verified(), (820., 640.));
    Fixture::click(&mut agents, "Agents");
    assert!(
        Fixture::label(&agents, "Add to Codex")
            .unwrap()
            .is_visible()
    );
    Fixture::screenshot(&mut agents, format!("{directory}/agents-codex-compact.png"));
    Fixture::click(&mut agents, "DeepSeek Harness");
    Fixture::screenshot(&mut agents, format!("{directory}/agents-compact.png"));
    let mut initial = Fixture::verified();
    initial.workspaces[0].preference = Ok(crate::preferences::WorkingDirectoryPreference::Custom(
        "/mnt/project".parse().unwrap(),
    ));
    let app = Shroom {
        initial,
        ..Shroom::default()
    };
    app.backend.agents.preview(
        "project",
        shroom_integrations::WebApp::Kimi,
        crate::agents::TunnelState::Ready("http://127.0.0.1:5494/?token=preview".into()),
    );
    let mut agents = Fixture::with_app(app, (1120., 800.));
    Fixture::click(&mut agents, "Agents");
    Fixture::click(&mut agents, "Kimi");
    Fixture::screenshot(&mut agents, format!("{directory}/agents-ready.png"));
}

#[test]
fn creation_can_add_toggle_and_remove_shared_folders() {
    let mut runner = Fixture::runner(Fixture::workspaces(SandboxStatus::Stopped), (1120., 1200.));
    Fixture::click(&mut runner, "New workspace");
    assert!(Fixture::label(&runner, "Guest username").is_some());
    Fixture::click(&mut runner, "Add shared folder");
    assert!(Fixture::label(&runner, "Host folder").is_some());
    assert!(Fixture::paragraph(&runner, "/mnt/shared-1").is_some());
    Fixture::click(&mut runner, "Read only");
    assert!(Fixture::label(&runner, "Read & write").is_some());
    Fixture::click(&mut runner, "Read & write");
    assert!(Fixture::label(&runner, "Read only").is_some());
    Fixture::click(&mut runner, "Add shared folder");
    assert!(Fixture::paragraph(&runner, "/mnt/shared-2").is_some());
    Fixture::click(&mut runner, "Remove folder");
    assert!(Fixture::paragraph(&runner, "/mnt/shared-1").is_none());
    assert!(Fixture::paragraph(&runner, "/mnt/shared-2").is_some());
    Fixture::click(&mut runner, "Add shared folder");
    assert!(Fixture::paragraph(&runner, "/mnt/shared-1").is_some());
    if let Ok(directory) = std::env::var("SHROOM_PREVIEW_DIR") {
        Fixture::screenshot(&mut runner, format!("{directory}/creation-options.png"));
    }
}
