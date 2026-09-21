use shroom_integrations::{GuestPort, HostKeyHandling, NativeApp, WebApp};

use super::*;
use crate::{
    agents::{LaunchState, TunnelState},
    model::{Error, WebForward},
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Agent {
    Native(NativeApp),
    Web(WebApp),
}

impl Default for Agent {
    fn default() -> Self {
        Self::Native(NativeApp::Codex)
    }
}

impl Agent {
    const ALL: [Self; 6] = [
        Self::Native(NativeApp::Codex),
        Self::Native(NativeApp::ClaudeDesktop),
        Self::Native(NativeApp::Cursor),
        Self::Native(NativeApp::ZCode),
        Self::Web(WebApp::Kimi),
        Self::Web(WebApp::DeepSeekHarness),
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Native(app) => app.name(),
            Self::Web(app) => app.name(),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Native(_) => Icon::Laptop,
            Self::Web(_) => Icon::Globe,
        }
    }
}

impl Ui {
    pub(super) fn agent_panel(
        &self,
        view: &Model,
        nav: &Navigation,
        workspace: &WorkspaceView,
    ) -> Rect {
        let p = self.colors;
        let mut navigation = self.navigation;
        rect()
            .width(Size::fill())
            .spacing(16.)
            .child(p.caption("Choose an app to work in this workspace."))
            .children(Agent::ALL.chunks(3).map(|row| {
                rect()
                    .horizontal()
                    .width(Size::fill())
                    .content(Content::Flex)
                    .spacing(6.)
                    .children(row.iter().copied().map(|agent| {
                        p.button_icon(agent.name(), agent.icon(), true)
                            .width(Size::flex(1.))
                            .padding((10., 6.))
                            .background(if nav.agent == agent {
                                p.selected
                            } else {
                                p.background
                            })
                            .on_press(move |_| navigation.write().agent = agent)
                            .into_element()
                    }))
                    .into_element()
            }))
            .child(p.divider())
            .child(match nav.agent {
                Agent::Native(app) => self.native_agent(view, workspace, app).into_element(),
                Agent::Web(app) => WebAgentPanel {
                    ui: self.clone(),
                    name: workspace.name.clone(),
                    app,
                    key: DiffKey::from(&format!(
                        "{:?}/{}/{:?}/{:?}",
                        view.session.as_ref().map(|session| &session.state_dir),
                        workspace.name,
                        app,
                        workspace
                            .ssh
                            .as_ref()
                            .map(|ssh| (&ssh.endpoint, &ssh.host_key_alias))
                    )),
                }
                .into_element(),
            })
    }

    fn native_agent(&self, view: &Model, workspace: &WorkspaceView, app: NativeApp) -> Rect {
        let p = self.colors;
        let direct = app.host_key_handling() == HostKeyHandling::ClientVerificationRequired;
        let format = if direct {
            ExportFormat::ConnectionDetails
        } else {
            ExportFormat::Config
        };
        let codex = app == NativeApp::Codex;
        let export: crate::model::Result<(String, String)> = if codex {
            workspace.project().and_then(|project| {
                let attachment = crate::codex::Codex::attachment(
                    &workspace.name,
                    project.attachment.connection().clone(),
                )?;
                Ok((attachment.alias().to_string(), attachment.ssh_config()))
            })
        } else {
            ConnectionExport::with_workspace(workspace, format)
                .map(|export| (format!("shroom-{}", workspace.name), export.text))
        };
        let enabled = view.available() && export.is_ok();
        let alias = export
            .as_ref()
            .map(|(alias, _)| alias.clone())
            .unwrap_or_default();
        let action = if codex {
            Command::AddToCodex(workspace.name.clone())
        } else {
            Command::Export(workspace.name.clone(), format)
        };
        let primary_action = || {
            p.primary(self.command_icon(
                if codex {
                    "Add to Codex"
                } else {
                    format.copy_label()
                },
                if codex {
                    Icon::ExternalLink
                } else {
                    Icon::Copy
                },
                action.clone(),
                enabled,
            ))
        };
        rect().width(Size::fill()).spacing(14.)
            .child(label().text(format!("Connect with {}", app.name())).font_weight(FontWeight::MEDIUM))
            .child(label().text(if codex {
                "Install Codex in this workspace if needed, then add its project and SSH connection to Codex. The first install needs internet access and may take a few minutes. Finish any sign-in in Codex."
            } else { app.setup() }).font_size(13.).color(p.muted))
            .maybe_child(codex.then(&primary_action))
            .maybe_child(direct.then(|| {
                Icon::ShieldQuestion.beside(label().text("Host-key verification in ZCode is not yet validated.")
                    .font_size(12.)).color(p.warning)
            }))
            .maybe_child((!direct && enabled).then(|| {
                rect().horizontal().width(Size::fill()).content(Content::Flex).spacing(16.)
                    .child(rect().width(Size::flex(1.)).child(p.value("SSH host alias", alias)))
                    .child(rect().width(Size::flex(1.)).child(p.value("Remote project folder", workspace.directory().map(|directory| directory.to_string()).unwrap_or_else(|error| error.to_string()))))
            }))
            .child(ScrollView::new().width(Size::fill()).height(Size::px(if direct { 172. } else { 110. }))
                .child(rect().width(Size::fill()).padding(10.).background(p.surface)
                    .child(p.code(match export {
                        Ok(export) if view.available() => export.1,
                        Ok(_) => "Connection details are unavailable while refreshing or changing this workspace.".into(),
                        Err(error) => error.to_string(),
                    }))))
            .maybe_child((!codex).then(primary_action))
            .child(self.external_link("Setup documentation", app.documentation().into(), true))
    }

    fn external_link(&self, label: &str, url: String, enabled: bool) -> Button {
        let ui = self.clone();
        self.colors
            .button_icon(label, Icon::ExternalLink, enabled)
            .flat()
            .on_press(move |_| {
                let mut ui = ui.clone();
                let url = url.clone();
                spawn_forever(async move {
                    // The URL is a documented HTTPS destination or a validated, checked loopback URL.
                    // Do not include it in diagnostics: a web login URL may contain a token.
                    if !matches!(
                        tokio::task::spawn_blocking(move || open::that(url)).await,
                        Ok(Ok(()))
                    ) {
                        ui.model
                            .write()
                            .errors
                            .push("Could not open your default browser.".into());
                    }
                });
            })
    }
}

#[derive(PartialEq)]
struct WebAgentPanel {
    ui: Ui,
    name: WorkspaceName,
    app: WebApp,
    key: DiffKey,
}

impl Component for WebAgentPanel {
    fn render_key(&self) -> DiffKey {
        self.key.clone()
    }

    fn render(&self) -> impl IntoElement {
        let p = self.ui.colors;
        let port = self.app.default_port().to_string();
        let requested_port = use_state(|| port.clone());
        let local_port = use_state(|| port);
        let reported_url = use_state(String::new);
        let mut show_output = use_state(|| None);
        let view = self.ui.model.read();
        let workspace = view
            .workspaces
            .iter()
            .find(|workspace| workspace.name == self.name);
        let available =
            view.available() && workspace.is_some_and(|workspace| workspace.ssh.is_some());
        let sessions = self.ui.web_sessions.read();
        let session = sessions
            .iter()
            .find(|session| session.name == self.name && session.app == self.app);
        let current = session.filter(|session| {
            workspace
                .is_some_and(|workspace| ConnectionExport::matches(&session.connection, workspace))
        });
        let directory = workspace.map(|workspace| workspace.directory());
        let can_launch = available
            && directory
                .as_ref()
                .is_some_and(|directory| directory.is_ok());
        let running = current.is_some_and(|session| session.launch == LaunchState::Running);
        let ui = self.ui.clone();
        let name = self.name.clone();
        let app = self.app;
        rect().width(Size::fill()).spacing(14.)
            .child(rect().horizontal().width(Size::fill()).content(Content::Flex).cross_align(Alignment::Center).spacing(10.)
                .child(rect().width(Size::flex(1.)).child(label().text(format!("{} in your browser", app.name())).font_weight(FontWeight::MEDIUM)))
                .child(self.ui.external_link("Docs", app.documentation().into(), true)))
            .child(p.caption(format!("Install {} in this workspace first. Authenticate using the app's normal setup.", app.executable())))
            .maybe_child(directory.as_ref().map(|directory| match directory {
                Ok(directory) => p.value("Default working directory", directory.to_string()).into_element(),
                Err(error) => p.caption(error.to_string()).color(p.error).into_element(),
            }))
            .maybe_child((!running).then(|| {
                rect().horizontal().width(Size::fill()).content(Content::Flex).cross_align(Alignment::End).spacing(10.)
                    .child(rect().width(Size::flex(1.)).child(p.field("Requested guest port", "5494", requested_port.into(), available)))
                    .child(p.primary(p.button_icon(format!("Launch {}", app.name()), Icon::Play, can_launch)
                        .on_press(move |_| {
                            let command = requested_port.peek().parse::<u32>().map_err(|_| Error::InvalidWebPort)
                                .and_then(|port| GuestPort::try_from(port).map_err(Error::from))
                                .map(|port| Command::LaunchWeb(name.clone(), app, port));
                            ui.clone().submit(command);
                        })))
            }))
            .maybe_child(current.map(|session| {
                let output = session.output.clone();
                let output_open = show_output().unwrap_or(!matches!(session.tunnel, TunnelState::Ready(_)));
                rect().width(Size::fill()).spacing(12.)
                    .child(p.value("Session directory", session.directory.to_string()))
                    .child(Icon::Globe.beside(label().text(session.status()).font_size(13.))
                        .color(if session.launch == LaunchState::Exited || matches!(session.tunnel, TunnelState::Failed(_)) { p.warning } else { p.green }))
                    .child(p.button(if output_open { "Hide launch output" } else { "Show launch output" }, true).flat()
                        .on_press(move |_| show_output.set(Some(!output_open))))
                    .maybe_child(output_open.then(|| {
                        ScrollView::new().width(Size::fill()).height(Size::px(125.))
                            .child(rect().width(Size::fill()).padding(10.).background(p.surface)
                                .child(p.code(if output.is_empty() { "Waiting for app output…".into() } else { output })))
                    }))
                    .maybe_child((running && !matches!(session.tunnel, TunnelState::Ready(_))).then(|| {
                        let ui = self.ui.clone();
                        let name = self.name.clone();
                        rect().width(Size::fill()).spacing(10.)
                            .child(p.caption("Paste the HTTP URL printed above, including any login token. The app may choose a different guest port."))
                            .child(p.field("Reported app URL", "http://127.0.0.1:5494/", reported_url.into(), available))
                            .child(p.field("Local browser port", "5494", local_port.into(), available))
                            .child(p.primary(p.button_icon("Connect web app", Icon::Globe, available)
                                .on_press(move |_| ui.clone().submit(WebForward::parse(&reported_url.peek(), &local_port.peek())
                                    .map(|forward| Command::ForwardWeb(name.clone(), app, forward))))))
                    }))
                    .maybe_child(match &session.tunnel {
                        TunnelState::Failed(error) => Some(label().text(error.clone()).font_size(13.).color(p.error).into_element()),
                        TunnelState::Ready(url) if available => {
                            let text = url.clone();
                            let mut model = self.ui.model;
                            Some(rect().width(Size::fill()).spacing(10.)
                                .child(p.code(url.clone()))
                                .child(p.primary(self.ui.external_link("Open in browser", url.clone(), available)))
                                .child(p.button_icon("Copy browser URL", Icon::Copy, available).on_press(move |_| {
                                    match Clipboard::set(text.clone()) {
                                        Ok(()) => model.write().notice = Some("Browser URL copied".into()),
                                        Err(_) => model.write().errors.push("Clipboard unavailable. Select and copy the displayed URL.".into()),
                                    }
                                })).into_element())
                        }
                        _ => None,
                    })
                    .child(self.ui.command_icon("Close web connection", Icon::Stop,
                        Command::CloseWeb(self.name.clone(), app), view.pending.is_none()))
                    .child(p.caption("Closing disconnects Shroom's SSH launch and tunnel. The guest app may keep running; the workspace stays running."))
            }))
    }
}
