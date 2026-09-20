use std::{
    fs::{self, File, OpenOptions},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use microsandbox::{
    Backend, LocalBackend, NetworkPolicy, NetworkProfile, Sandbox, SandboxConfig,
    config::{GlobalConfig, PathsConfig},
    sandbox::{PortProtocol, PullPolicy, RootDisk, RootfsSource, SandboxHandle},
    setup,
};
use tokio::time;

use crate::{
    Config, Error, FolderAccess, HostPort, HostPublicKey, MicrosandboxError, RUNTIME_VERSION,
    Result, SandboxStatus, Stage, WORKSPACE_IMAGE, Workspace, WorkspaceName, WorkspaceOptions,
    access::{Access, GUEST_HELPER, GUEST_HOST_KEY, Helper, Material},
};

const BOOT_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const ADMIN_TIMEOUT: Duration = Duration::from_secs(15);

/// A private SDK catalog and its SSH access artifacts, protected by one OS file lock.
/// Mutations require `&mut self`; dropping the core leaves detached VMs running.
pub struct Core {
    backend: Arc<LocalBackend>,
    access_dir: PathBuf,
    _lock: StateLock,
}

struct StateLock(File);

impl StateLock {
    fn new(state_dir: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(state_dir.join("core.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Self(file)),
            Err(fs::TryLockError::WouldBlock) => Err(Error::CoreInUse(state_dir.to_owned())),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        // A concurrent fork can temporarily inherit the open file description until exec.
        // Explicit unlock releases ownership without waiting for those descriptors to close.
        let _ = self.0.unlock();
    }
}

impl Core {
    /// Acquire the root lock and validate the preinstalled runtime and host SSH tools.
    /// The image must be imported separately; opening starts no sandboxes.
    pub async fn open(state_dir: PathBuf, config: Config) -> Result<Self> {
        fs::create_dir_all(&state_dir)?;
        let state_dir = state_dir.canonicalize()?;
        if state_dir
            .to_str()
            .is_none_or(|path| path.contains('$') || path.chars().any(char::is_control))
        {
            return Err(MicrosandboxError::InvalidConfig(
                "state path must be UTF-8 without control characters or OpenSSH environment expansion".into()).into());
        }
        let lock = StateLock::new(&state_dir)?;
        if !cfg!(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64")
        )) {
            return Err(MicrosandboxError::InvalidConfig(
                "Shroom supports macOS ARM64 and Linux x86-64".into(),
            )
            .into());
        }
        Helper::prerequisites().await?;
        let backend = config.backend(&state_dir).await?;
        let access_dir = state_dir.join("access");
        Access::create_directory(&access_dir)?;
        Ok(Self {
            backend: Arc::new(backend),
            access_dir,
            _lock: lock,
        })
    }

    /// Create once, provision once, pin the guest identity through SDK administration, then authenticate.
    /// Errors and cancellation retain partial artifacts for explicit [`Self::remove`].
    pub async fn create(
        &mut self,
        name: WorkspaceName,
        host_ssh_port: HostPort,
        options: WorkspaceOptions,
    ) -> Result<Workspace> {
        let access = self.access(&name);
        self.scoped(async {
            match Sandbox::get(name.as_str()).await {
                Ok(_) => {
                    return Err(Stage::Validate.context(
                        &name,
                        MicrosandboxError::SandboxAlreadyExists(name.to_string()),
                    ));
                }
                Err(MicrosandboxError::SandboxNotFound(_)) => (),
                Err(error) => return Err(Stage::Validate.context(&name, error)),
            }
            if access
                .exists()
                .map_err(|e| Stage::Validate.context(&name, e))?
            {
                return Err(
                    Stage::Validate.context(&name, Error::AccessConflict(access.directory.clone()))
                );
            }
            self.check_port(host_ssh_port)
                .await
                .map_err(|e| Stage::Validate.context(&name, e))?;
            options
                .validate_sources(self.access_dir.parent().expect("state directory"))
                .map_err(|e| Stage::Validate.context(&name, e))?;
            let config = Self::sandbox_config(&name, host_ssh_port, &options)
                .await
                .map_err(|e| Stage::Validate.context(&name, e))?;
            let client = access
                .create()
                .await
                .map_err(|e| Stage::CreateAccess.context(&name, e))?;
            let sandbox = time::timeout(BOOT_TIMEOUT, Sandbox::create_detached(config))
                .await
                .map_err(|e| Stage::CreateSandbox.context(&name, e))?
                .map_err(|e| Stage::CreateSandbox.context(&name, e))?;
            let public = client
                .to_openssh()
                .map_err(|e| Stage::Provision.context(&name, e))?;
            Self::launch_ssh(&sandbox, Some((public, &options)))
                .await
                .map_err(|e| Stage::Provision.context(&name, e))?;
            let host_key =
                time::timeout(ADMIN_TIMEOUT, sandbox.fs().read_to_string(GUEST_HOST_KEY))
                    .await
                    .map_err(|e| Stage::PinHostKey.context(&name, e))?
                    .map_err(|e| Stage::PinHostKey.context(&name, e))?;
            let host_key: HostPublicKey = host_key
                .parse()
                .map_err(|e| Stage::PinHostKey.context(&name, e))?;
            access
                .pin(&host_key)
                .map_err(|e| Stage::PinHostKey.context(&name, e))?;
            let handle = Sandbox::get(name.as_str())
                .await
                .map_err(|e| Stage::ReadCatalog.context(&name, e))?;
            Self::ready(&name, &handle, &access, Material { host_key }).await
        })
        .await
    }

    /// Read every catalog page, including stopped and partially created workspaces.
    pub async fn list(&self) -> Result<Vec<Workspace>> {
        self.scoped(async {
            let handles = Self::handles().await?;
            handles
                .iter()
                .map(|handle| self.workspace(handle))
                .collect()
        })
        .await
    }

    /// Inspect catalog and access files without starting, provisioning, or probing.
    pub async fn get(&self, name: &WorkspaceName) -> Result<Workspace> {
        self.scoped(async {
            let handle = Sandbox::get(name.as_str())
                .await
                .map_err(|e| Stage::ReadCatalog.context(name, e))?;
            self.workspace(&handle)
        })
        .await
    }

    /// Start the same persisted VM in detached mode and verify the original credentials.
    /// An already running VM is probed without relaunching its SSH service.
    pub async fn start(&mut self, name: &WorkspaceName) -> Result<Workspace> {
        self.scoped(async {
            let handle = Sandbox::get(name.as_str())
                .await
                .map_err(|e| Stage::ReadCatalog.context(name, e))?;
            let access = self.access(name);
            let material = access
                .read()
                .map_err(|e| Stage::ReadAccess.context(name, e))?
                .ok_or_else(|| {
                    Stage::ReadAccess
                        .context(name, Error::AccessIncomplete(access.directory.clone()))
                })?;
            let config = handle
                .config()
                .map_err(|e| Stage::Validate.context(name, e))?;
            Self::validate_config(&config).map_err(|e| Stage::Validate.context(name, e))?;
            WorkspaceOptions::from_config(&config)?
                .validate_sources(self.access_dir.parent().expect("state directory"))
                .map_err(|e| Stage::Validate.context(name, e))?;
            let newly_booted = handle.status_snapshot() != SandboxStatus::Running;
            let sandbox = time::timeout(BOOT_TIMEOUT, handle.connect_or_start_detached())
                .await
                .map_err(|e| Stage::Start.context(name, e))?
                .map_err(|e| Stage::Start.context(name, e))?;
            if newly_booted {
                Self::launch_ssh(&sandbox, None)
                    .await
                    .map_err(|e| Stage::Start.context(name, e))?;
            }
            Self::ready(name, &handle, &access, material).await
        })
        .await
    }

    /// Request graceful shutdown. A timeout never requests force termination.
    pub async fn stop(&mut self, name: &WorkspaceName) -> Result<()> {
        self.scoped(async {
            let handle = Sandbox::get(name.as_str())
                .await
                .map_err(|e| Stage::Stop.context(name, e))?;
            handle
                .stop_with_timeout(STOP_TIMEOUT)
                .await
                .map_err(|e| Stage::Stop.context(name, e))
        })
        .await
    }

    /// Let the SDK verify runtime quiescence and remove the VM before deleting access files.
    /// Repeating removal can finish access cleanup after the VM has already disappeared.
    pub async fn remove(&mut self, name: &WorkspaceName) -> Result<()> {
        self.scoped(async {
            match Sandbox::get(name.as_str()).await {
                Ok(handle) => handle
                    .remove()
                    .await
                    .map_err(|e| Stage::RemoveSandbox.context(name, e))?,
                Err(MicrosandboxError::SandboxNotFound(_)) => (),
                Err(error) => return Err(Stage::RemoveSandbox.context(name, error)),
            }
            self.access(name)
                .remove()
                .map_err(|e| Stage::RemoveAccess.context(name, e))
        })
        .await
    }

    fn scoped<F: std::future::Future>(
        &self,
        future: F,
    ) -> impl std::future::Future<Output = F::Output> {
        // SDK operation frames contain large native configurations. Keep them off callers' stacks.
        microsandbox::with_backend(self.backend.clone() as Arc<dyn Backend>, Box::pin(future))
    }

    fn access(&self, name: &WorkspaceName) -> Access {
        Access {
            directory: self.access_dir.join(name.as_str()),
        }
    }

    async fn handles() -> Result<Vec<SandboxHandle>> {
        let mut page = Sandbox::list().await?;
        let mut handles = page.sandboxes;
        while let Some(cursor) = page.next_cursor {
            page = Sandbox::list_with(|list| list.cursor(cursor)).await?;
            handles.extend(page.sandboxes);
        }
        Ok(handles)
    }

    async fn check_port(&self, port: HostPort) -> Result<()> {
        for handle in Self::handles().await? {
            if handle
                .config()?
                .spec
                .network
                .ports
                .iter()
                .any(|p| p.host_port == port.get())
            {
                return Err(Error::PortInUse {
                    port,
                    workspace: handle.name().parse()?,
                });
            }
        }
        Ok(())
    }

    fn workspace(&self, handle: &SandboxHandle) -> Result<Workspace> {
        let name: WorkspaceName = handle.name().parse()?;
        let options = WorkspaceOptions::from_config(&handle.config()?)?;
        let access = self.access(&name);
        let material = access
            .read()
            .map_err(|e| Stage::ReadAccess.context(&name, e))?;
        let endpoint =
            Self::endpoint(handle).map_err(|e| Stage::DiscoverEndpoint.context(&name, e))?;
        let ssh = material
            .zip(endpoint)
            .map(|(material, endpoint)| access.connection(material, endpoint, &options.user));
        Ok(Workspace {
            name,
            state: handle.status_snapshot(),
            options,
            ssh,
        })
    }

    fn endpoint(handle: &SandboxHandle) -> Result<Option<SocketAddr>> {
        if handle.status_snapshot() != SandboxStatus::Running {
            return Ok(None);
        }
        // Read the active run's SDK mapping; never cache an address in Shroom's access files.
        let Some(config) = handle.active_config()? else {
            return Ok(None);
        };
        let Some(port) = config
            .spec
            .network
            .ports
            .iter()
            .find(|port| port.guest_port == 22 && port.protocol == PortProtocol::Tcp)
        else {
            return Ok(None);
        };
        let ip: IpAddr = port.host_bind.parse().map_err(|_| {
            MicrosandboxError::InvalidConfig("invalid published SSH address".into())
        })?;
        if ip != IpAddr::V4(Ipv4Addr::LOCALHOST) || port.host_port < 1024 {
            return Err(MicrosandboxError::InvalidConfig(
                "SSH must publish on unprivileged host loopback".into(),
            )
            .into());
        }
        Ok(Some(SocketAddr::new(ip, port.host_port)))
    }

    async fn ready(
        name: &WorkspaceName,
        handle: &SandboxHandle,
        access: &Access,
        material: Material,
    ) -> Result<Workspace> {
        let endpoint = time::timeout(ADMIN_TIMEOUT, async {
            loop {
                let current = handle.refresh().await?;
                if let Some(endpoint) = Self::endpoint(&current)? {
                    break Ok::<_, Error>(endpoint);
                }
                time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|e| Stage::DiscoverEndpoint.context(name, e))?
        .map_err(|e| Stage::DiscoverEndpoint.context(name, e))?;
        let options = WorkspaceOptions::from_config(&handle.config()?)?;
        let ssh = access.connection(material, endpoint, &options.user);
        ssh.probe()
            .await
            .map_err(|e| Stage::Probe.context(name, e))?;
        Ok(Workspace {
            name: name.clone(),
            state: SandboxStatus::Running,
            options,
            ssh: Some(ssh),
        })
    }

    async fn launch_ssh(
        sandbox: &Sandbox,
        provision: Option<(String, &WorkspaceOptions)>,
    ) -> Result<()> {
        if provision.is_some() {
            time::timeout(
                ADMIN_TIMEOUT,
                sandbox.fs().write(
                    GUEST_HELPER,
                    include_bytes!("../../../images/workspace/shroom-ssh"),
                ),
            )
            .await??;
        }
        let output = time::timeout(
            ADMIN_TIMEOUT,
            sandbox.exec_with("/bin/sh", |exec| {
                let exec = exec
                    .arg(GUEST_HELPER)
                    .user("root")
                    .cwd("/")
                    .timeout(Duration::from_secs(10));
                match provision {
                    Some((public, options)) => exec
                        .args(["provision", options.user.as_str()])
                        .stdin_bytes(format!("{public}\n")),
                    None => exec.arg("start").stdin_null(),
                }
            }),
        )
        .await??;
        if !output.status().success {
            return Err(Error::GuestHelperFailed {
                code: output.status().code,
                diagnostic: Helper::diagnostic(output.stderr_bytes()),
            });
        }
        Ok(())
    }

    fn network_policy() -> NetworkPolicy {
        NetworkPolicy::from_profiles([NetworkProfile::Public])
    }

    async fn sandbox_config(
        name: &WorkspaceName,
        port: HostPort,
        options: &WorkspaceOptions,
    ) -> Result<SandboxConfig> {
        options.validate()?;
        let builder = Sandbox::builder(name.as_str())
            .image(WORKSPACE_IMAGE)
            .pull_policy(PullPolicy::Never)
            .cpus(2)
            .memory(4096_u32)
            .root_disk(4096_u32)
            .shell("/bin/bash")
            .user("root")
            // The account's home is created during provisioning; administration boots at /.
            .workdir("/")
            .label(WorkspaceOptions::USER_LABEL, options.user.as_str())
            .network(|network| network.policy(Self::network_policy()))
            .port_bind(Ipv4Addr::LOCALHOST.into(), port.get(), 22);
        let config = options
            .folders
            .iter()
            .fold(builder, |builder, folder| {
                builder.volume(folder.guest(), |mount| {
                    let mount = mount.bind(folder.host()).owner(1000, 1000).nosuid().nodev();
                    match folder.access() {
                        FolderAccess::ReadOnly => mount.readonly(),
                        FolderAccess::ReadWrite => mount,
                    }
                })
            })
            .build()
            .await?;
        Self::validate_config(&config)?;
        if !WorkspaceOptions::from_config(&config)?.matches(options) {
            return Err(Error::InvalidHostFolder(
                "effective SDK configuration changed the selected account or folders",
            ));
        }
        Ok(config)
    }

    fn validate_config(config: &SandboxConfig) -> Result<()> {
        WorkspaceOptions::from_config(config)?;
        let spec = &config.spec;
        let network = &spec.network;
        // The SDK exposes engine and wire policy types separately; compare their wire representation.
        let policy = serde_json::to_value(&network.policy).map_err(MicrosandboxError::from)?;
        let expected =
            serde_json::to_value(Self::network_policy()).map_err(MicrosandboxError::from)?;
        let private_root = matches!(&spec.image, RootfsSource::Oci(image)
            if image.reference == WORKSPACE_IMAGE
                && matches!(image.root_disk, None | Some(RootDisk::Managed { .. })));
        if !private_root
            || !spec.patches.is_empty()
            || !spec.vsock.is_empty()
            || spec.init.is_some()
            || !spec.runtime.scripts.is_empty()
            || spec.env.iter().any(|entry| entry.key != "PATH")
            || spec.resources.cpus != 2
            || spec.resources.memory_mib != 4096
            || spec.lifecycle.ephemeral
            || spec.lifecycle.idle_timeout_secs.is_some()
            || spec.lifecycle.max_duration_secs.is_some()
            || !network.enabled
            || network.trust_host_cas
            || network.outbound_proxy.is_some()
            || network
                .secrets
                .as_ref()
                .is_some_and(|secrets| !secrets.secrets.is_empty())
            || network.tls.as_ref().is_some_and(|tls| tls.enabled)
            || network
                .dns
                .as_ref()
                .is_some_and(|dns| !dns.rebind_protection)
            || policy != expected
            || network.ports.len() != 1
            || network.ports.iter().any(|p| {
                p.host_bind != "127.0.0.1"
                    || p.host_port < 1024
                    || p.guest_port != 22
                    || p.protocol != PortProtocol::Tcp
            })
        {
            return Err(MicrosandboxError::InvalidConfig(
                "effective SDK configuration conflicts with the fixed Shroom guest profile or isolation".into()).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod runtime_tests;

impl Config {
    async fn backend(&self, state_dir: &Path) -> Result<LocalBackend> {
        let runtime_executable = self.runtime_executable.canonicalize()?;
        let firmware = self.firmware.canonicalize()?;
        let home = state_dir.join("microsandbox");
        fs::create_dir_all(&home)?;
        let config_path = home.join("config.json");
        let config = GlobalConfig {
            home: Some(home.clone()),
            paths: PathsConfig {
                msb: Some(runtime_executable.clone()),
                libkrunfw: Some(firmware.clone()),
                ..Default::default()
            },
            ..Default::default()
        };
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).map_err(MicrosandboxError::from)?,
        )?;
        // Lazy construction lets us reject machine/environment path conflicts before the SDK opens a DB.
        let backend = LocalBackend::builder()
            .home(&home)
            .config_path(config_path)
            .build_lazy()?;
        let effective = backend.config();
        let runtime = setup::resolve_runtime(effective)?;
        if effective.home() != home
            || effective.paths.agentd.is_some()
            || [
                effective.sandboxes_dir(),
                effective.cache_dir(),
                effective.volumes_dir(),
                effective.snapshots_dir(),
                effective.logs_dir(),
                effective.secrets_dir(),
            ]
            .iter()
            .any(|path| {
                !path.starts_with(&home) || path.components().any(|c| c == Component::ParentDir)
            })
            || runtime.msb_path.canonicalize()? != runtime_executable
            || runtime.libkrunfw_path.canonicalize()? != firmware
        {
            return Err(MicrosandboxError::InvalidConfig(
                "effective SDK paths conflict with the private Shroom root or configured runtime"
                    .into(),
            )
            .into());
        }
        let version = setup::resolve_runtime_version(&runtime.msb_path)?;
        if version.as_ref().map(ToString::to_string).as_deref() != Some(RUNTIME_VERSION) {
            return Err(MicrosandboxError::InvalidConfig(format!(
                "Shroom requires microsandbox {RUNTIME_VERSION}; found {version:?}"
            ))
            .into());
        }
        time::timeout(ADMIN_TIMEOUT, backend.db()).await??;
        Ok(backend)
    }
}
