use std::{
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use shroom_core::{HostPort, SshConnection, Workspace, WorkspaceName};
use shroom_integrations::{Attachment, GuestPort, GuestWebUrl, WebApp, WebTunnel};
use tokio::{
    io::AsyncReadExt,
    process::Child,
    sync::{Mutex, oneshot, watch},
    task::JoinHandle,
    time,
};

use crate::model::ConnectionExport;

const OUTPUT_LIMIT: usize = 32 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("This web launch is already running. Close its connection before launching again.")]
    AlreadyRunning,
    #[error("Launch the web app first, then paste the URL it reports.")]
    NotRunning,
    #[error("The workspace connection changed. Close this launch and start it again.")]
    ConnectionChanged,
    #[error(
        "SSH could not establish the tunnel. Check the output and choose an unused local port."
    )]
    TunnelUnavailable,
    #[error(
        "The web app did not respond through the tunnel. Check its reported URL and try again."
    )]
    HttpUnavailable,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchState {
    Running,
    Exited,
}

#[derive(Clone)]
pub enum TunnelState {
    Idle,
    Connecting,
    Ready(String),
    Failed(String),
}

#[derive(Clone)]
pub struct WebSession {
    id: u64,
    pub name: WorkspaceName,
    pub app: WebApp,
    pub connection: SshConnection,
    pub launch: LaunchState,
    pub tunnel: TunnelState,
    /// Bounded, memory-only output. It can contain application login URLs.
    pub output: String,
}

impl WebSession {
    pub fn status(&self) -> &'static str {
        if self.launch == LaunchState::Exited {
            return "Launch session ended";
        }
        match self.tunnel {
            TunnelState::Idle => "Waiting for the app's URL",
            TunnelState::Connecting => "Connecting to web app…",
            TunnelState::Ready(_) => "Web app reachable",
            TunnelState::Failed(_) => "Connection needs attention",
        }
    }
}

pub struct AgentManager {
    updates: watch::Sender<Vec<WebSession>>,
    sessions: Mutex<Vec<ManagedSession>>,
    next_id: AtomicU64,
}

impl Default for AgentManager {
    fn default() -> Self {
        Self {
            updates: watch::channel(Vec::new()).0,
            sessions: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }
}

impl AgentManager {
    pub fn subscribe(&self) -> watch::Receiver<Vec<WebSession>> {
        self.updates.subscribe()
    }

    pub async fn launch(
        &self,
        name: WorkspaceName,
        app: WebApp,
        attachment: Attachment,
        port: GuestPort,
    ) -> Result<(), Error> {
        if self.updates.borrow().iter().any(|session| {
            session.name == name && session.app == app && session.launch == LaunchState::Running
        }) {
            return Err(Error::AlreadyRunning);
        }
        self.close(Some((&name, Some(app)))).await;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let context = ProcessContext {
            id,
            role: ProcessRole::Launch,
            updates: self.updates.clone(),
        };
        self.updates.send_modify(|sessions| {
            sessions.push(WebSession {
                id,
                name: name.clone(),
                app,
                connection: attachment.connection().clone(),
                launch: LaunchState::Running,
                tunnel: TunnelState::Idle,
                output: String::new(),
            });
        });
        match ManagedProcess::spawn(app.launch_command(&attachment, port), context.clone()) {
            Ok((launch, _)) => {
                self.sessions.lock().await.push(ManagedSession {
                    id,
                    name,
                    app,
                    launch,
                    tunnel: None,
                });
                Ok(())
            }
            Err(error) => {
                context.finish("Could not launch host SSH.");
                Err(error.into())
            }
        }
    }

    pub async fn forward(
        &self,
        name: &WorkspaceName,
        app: WebApp,
        attachment: Attachment,
        guest: GuestWebUrl,
        local_port: HostPort,
    ) -> Result<(), Error> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .iter_mut()
            .find(|session| &session.name == name && session.app == app)
            .ok_or(Error::NotRunning)?;
        let context = ProcessContext {
            id: session.id,
            role: ProcessRole::Tunnel,
            updates: self.updates.clone(),
        };
        if !context.launch_running() {
            return Err(Error::NotRunning);
        }
        if !self.updates.borrow().iter().any(|view| {
            view.id == session.id
                && ConnectionExport::same_connection(&view.connection, attachment.connection())
        }) {
            return Err(Error::ConnectionChanged);
        }
        if let Some(tunnel) = session.tunnel.take() {
            tunnel.close().await;
        }
        context.update(|session| session.tunnel = TunnelState::Connecting);
        let tunnel = attachment.web_tunnel(guest, local_port);
        let outcome: Result<(), Error> = async {
            let (process, ready) =
                ManagedProcess::spawn(tunnel.command_with_readiness(), context.clone())?;
            session.tunnel = Some(process);
            time::timeout(Duration::from_secs(15), ready)
                .await
                .map_err(|_| Error::TunnelUnavailable)?
                .map_err(|_| Error::TunnelUnavailable)?;
            // The remote marker proves this SSH process established its forwards. A host listener
            // alone could be an unrelated process that already occupied the requested port.
            Self::probe(&tunnel, &context).await?;
            context.update(|session| {
                if session.launch == LaunchState::Running
                    && matches!(session.tunnel, TunnelState::Connecting)
                {
                    session.tunnel = TunnelState::Ready(tunnel.local_url().to_string());
                }
            });
            Ok(())
        }
        .await;
        if let Err(error) = &outcome {
            if let Some(tunnel) = session.tunnel.take() {
                tunnel.close().await;
            }
            context.update(|session| session.tunnel = TunnelState::Failed(error.to_string()));
        }
        outcome
    }

    async fn probe(tunnel: &WebTunnel, context: &ProcessContext) -> Result<(), Error> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|_| Error::HttpUnavailable)?;
        time::timeout(Duration::from_secs(8), async {
            loop {
                if !context.launch_running() || !context.tunnel_connecting() {
                    return Err(Error::TunnelUnavailable);
                }
                if client
                    .get(tunnel.local_url())
                    .send()
                    .await
                    .is_ok_and(|response| !response.status().is_server_error())
                {
                    return Ok(());
                }
                time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .unwrap_or(Err(Error::HttpUnavailable))
    }

    /// Drop only sessions whose connection snapshot is no longer current.
    pub async fn reconcile(&self, workspaces: &[Workspace]) {
        let expired = self
            .updates
            .borrow()
            .iter()
            .filter(|session| {
                !workspaces.iter().any(|workspace| {
                    workspace.name == session.name
                        && ConnectionExport::matches(&session.connection, workspace)
                })
            })
            .map(|session| (session.name.clone(), session.app))
            .collect::<Vec<_>>();
        for (name, app) in expired {
            self.close(Some((&name, Some(app)))).await;
        }
    }

    pub async fn close(&self, target: Option<(&WorkspaceName, Option<WebApp>)>) {
        let matches = |session: &ManagedSession| {
            target.is_none_or(|(name, app)| {
                &session.name == name && app.is_none_or(|app| session.app == app)
            })
        };
        let mut sessions = self.sessions.lock().await;
        let (closing, retained): (Vec<_>, Vec<_>) = sessions.drain(..).partition(matches);
        *sessions = retained;
        for session in closing {
            if let Some(tunnel) = session.tunnel {
                tunnel.close().await;
            }
            session.launch.close().await;
        }
        self.updates.send_modify(|sessions| {
            sessions.retain(|session| {
                !target.is_none_or(|(name, app)| {
                    &session.name == name && app.is_none_or(|app| session.app == app)
                })
            });
        });
    }
}

struct ManagedSession {
    id: u64,
    name: WorkspaceName,
    app: WebApp,
    launch: ManagedProcess,
    tunnel: Option<ManagedProcess>,
}

struct ManagedProcess {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl ManagedProcess {
    fn spawn(
        command: Command,
        context: ProcessContext,
    ) -> std::io::Result<(Self, oneshot::Receiver<()>)> {
        let child = tokio::process::Command::from(command)
            .kill_on_drop(true)
            .stdin(if context.role == ProcessRole::Tunnel {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (stop, cancel) = oneshot::channel();
        let (ready, receiver) = oneshot::channel();
        let task = tokio::spawn(Self::supervise(child, context, cancel, ready));
        Ok((Self { stop, task }, receiver))
    }

    async fn close(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }

    async fn supervise(
        mut child: Child,
        context: ProcessContext,
        mut cancel: oneshot::Receiver<()>,
        ready: oneshot::Sender<()>,
    ) {
        // Child::wait closes an attached stdin. Keep this pipe separately so the readiness
        // command's `cat` stays alive until the tunnel is explicitly closed.
        let _stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let mut out = [0; 4096];
        let mut err = [0; 4096];
        let mut out_open = true;
        let mut err_open = true;
        let mut out_text = OutputText::default();
        let mut err_text = OutputText::default();
        let mut marker = String::new();
        let mut ready = Some(ready);
        let mut changes = context.updates.subscribe();
        loop {
            tokio::select! {
                _ = &mut cancel => {
                    let _ = child.kill().await;
                    context.finish("SSH connection closed.");
                    break;
                }
                result = child.wait() => {
                    // Fast failures (for example a missing guest executable) can exit before the
                    // output branches run. Retain their final diagnostics without waiting forever.
                    context.output(&Self::tail(&mut stdout, &mut out_text).await);
                    context.output(&Self::tail(&mut stderr, &mut err_text).await);
                    context.finish(&match result {
                        Ok(status) => format!("SSH session ended ({status})."),
                        Err(_) => "Could not observe the SSH process.".into(),
                    });
                    break;
                }
                result = changes.changed(), if context.role == ProcessRole::Tunnel => {
                    if result.is_err() || !context.launch_running() {
                        let _ = child.kill().await;
                        context.finish("Launch session ended; tunnel closed.");
                        break;
                    }
                }
                result = stdout.read(&mut out), if out_open => {
                    if let Ok(count @ 1..) = result {
                        let text = out_text.decode(&out[..count]);
                        if context.role == ProcessRole::Tunnel && ready.is_some() {
                            OutputText::append(&mut marker, &text);
                            if marker.lines().any(|line| line == WebTunnel::READY_MESSAGE) {
                                let _ = ready.take().expect("pending readiness").send(());
                            }
                        } else if context.role == ProcessRole::Launch {
                            context.output(&text);
                        }
                    } else { out_open = false; }
                }
                result = stderr.read(&mut err), if err_open => {
                    if let Ok(count @ 1..) = result {
                        context.output(&err_text.decode(&err[..count]));
                    } else { err_open = false; }
                }
            }
        }
    }

    async fn tail(
        reader: &mut (impl tokio::io::AsyncRead + Unpin),
        decoder: &mut OutputText,
    ) -> String {
        let mut bytes = Vec::new();
        let _ = time::timeout(
            Duration::from_millis(100),
            reader.take(OUTPUT_LIMIT as u64).read_to_end(&mut bytes),
        )
        .await;
        decoder.decode(&bytes)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProcessRole {
    Launch,
    Tunnel,
}

#[derive(Clone)]
struct ProcessContext {
    id: u64,
    role: ProcessRole,
    updates: watch::Sender<Vec<WebSession>>,
}

impl ProcessContext {
    fn update(&self, update: impl FnOnce(&mut WebSession)) {
        self.updates.send_modify(|sessions| {
            if let Some(session) = sessions.iter_mut().find(|session| session.id == self.id) {
                update(session);
            }
        });
    }

    fn output(&self, text: &str) {
        self.update(|session| OutputText::append(&mut session.output, text));
    }

    fn launch_running(&self) -> bool {
        self.updates
            .borrow()
            .iter()
            .any(|session| session.id == self.id && session.launch == LaunchState::Running)
    }

    fn tunnel_connecting(&self) -> bool {
        self.updates.borrow().iter().any(|session| {
            session.id == self.id && matches!(session.tunnel, TunnelState::Connecting)
        })
    }

    fn finish(&self, message: &str) {
        self.update(|session| {
            OutputText::append(&mut session.output, &format!("\n{message}\n"));
            match self.role {
                ProcessRole::Launch => {
                    session.launch = LaunchState::Exited;
                    session.tunnel = TunnelState::Idle;
                }
                ProcessRole::Tunnel => session.tunnel = TunnelState::Failed(message.into()),
            }
        });
    }
}

#[derive(Default)]
enum EscapeState {
    #[default]
    Text,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

#[derive(Default)]
struct OutputText {
    escape: EscapeState,
}

impl OutputText {
    fn decode(&mut self, bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes)
            .chars()
            .filter(|&character| {
                match self.escape {
                    EscapeState::Text => match character {
                        '\u{1b}' => self.escape = EscapeState::Escape,
                        '\n' | '\t' => return true,
                        c if !c.is_control() => return true,
                        _ => (),
                    },
                    EscapeState::Escape => {
                        self.escape = match character {
                            '[' => EscapeState::Csi,
                            ']' => EscapeState::Osc,
                            _ => EscapeState::Text,
                        }
                    }
                    EscapeState::Csi if ('@'..='~').contains(&character) => {
                        self.escape = EscapeState::Text
                    }
                    EscapeState::Osc if character == '\u{7}' => self.escape = EscapeState::Text,
                    EscapeState::Osc if character == '\u{1b}' => {
                        self.escape = EscapeState::OscEscape
                    }
                    EscapeState::OscEscape => {
                        self.escape = if character == '\\' {
                            EscapeState::Text
                        } else {
                            EscapeState::Osc
                        }
                    }
                    _ => (),
                }
                false
            })
            .collect()
    }

    fn append(output: &mut String, text: &str) {
        output.push_str(text);
        if output.len() > OUTPUT_LIMIT {
            let mut boundary = output.len() - OUTPUT_LIMIT;
            while !output.is_char_boundary(boundary) {
                boundary += 1;
            }
            output.drain(..boundary);
        }
    }
}

#[cfg(test)]
mod tests;
