# Shroom

Shroom is an SSH-first workspace manager over microsandbox. The core exposes `create`, `list`, `get`,
`start`, `stop`, and `remove`, returning connection details for existing SSH clients.
See the [core contract](docs/references/shroom-core.md#ssh-workspace-core) and [documentation index](docs/README.md).

The repository contains a Freya desktop app in `crates/shroom-app`, the workspace library in `crates/shroom-core`,
agent app connection exports and launch recipes in `crates/shroom-integrations`, a typed Lume HTTP client in
`crates/shroom-lume`, a prepared microsandbox guest recipe in `images/workspace`, and documentation in `docs`.
It has no CLI or resident service of its own.

## Desktop App

Run the native interface with:

```sh
cargo run -p shroom-app --locked
```

Enter the state directory, microsandbox executable, and firmware paths, then choose **Open workspaces**.
Select a workspace in the sidebar to connect, start, stop, or delete it. The connection panel copies a full
SSH command or configuration and shows the last authenticated verification separately from runtime state.
Optional Details shows connection fields and a command for inspecting the same private catalog with `msb`.
The **Agents** tab can add a workspace directly to Codex through SSH. It also provides setup for
Claude Desktop, Cursor, and ZCode, plus managed SSH launches and browser tunnels for guest installations
of Kimi and DeepSeek Harness.
**Local workspaces** opens environment settings for image import and changing directories. Incomplete setups
appear under **Needs attention** for explicit cleanup. The app follows the system's light or dark appearance.
Runtime and image setup are described below. See the
[desktop app reference](docs/references/desktop-app.md) for configuration and behavior.

Freya's [development setup](https://docs.rs/freya/0.4.3/freya/_docs/development_setup/index.html)
lists the native build dependencies, including Linux packages. The first build downloads Skia artifacts.

## Development

The pinned Rust toolchain is installed through rustup. Dependencies live in the root workspace manifest.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Normal tests require host `ssh`, `ssh-keygen`, and permission to bind local TCP listeners, but do not boot VMs.
Real-runtime acceptance is opt-in; see [validation](docs/evaluations/2026-09-20-minimal-core/README.md).

## Local Runtime and Image

Install the matching microsandbox **0.7.2** executable and firmware before opening a core.
Pass the actual executable to `Config`, since SDK version verification reads its binary metadata.
For Homebrew installations, that is the `libexec/msb` file rather than the `bin/msb` shell wrapper.
The host also needs `ssh` and `ssh-keygen` on `PATH`.

Build the prepared image for the host's guest architecture using Docker or a compatible OCI builder.
Use `linux/arm64` for macOS ARM64 and `linux/amd64` for Linux x86-64:

```sh
docker build --platform linux/arm64 -t localhost/shroom-workspace:0.1.0 images/workspace
docker save -o /tmp/shroom-workspace.tar localhost/shroom-workspace:0.1.0
mkdir -p "$HOME/.shroom/microsandbox"
MSB_HOME="$HOME/.shroom/microsandbox" \
MSB_CONFIG_PATH="$HOME/.shroom/microsandbox/config.json" \
  msb image load -i /tmp/shroom-workspace.tar --tag localhost/shroom-workspace:0.1.0
```

Import the image before opening a core at the same state root. Shroom uses the fixed local image tag
with the SDK's `Never` pull policy. In the desktop app, **Import image** performs this import into the open
state directory; the CLI import above is an alternative. Runtime and image installation remain explicit setup steps.
Choose a short state path because the SDK derives Unix socket paths from it. Paths must be UTF-8
without control characters or `$`; spaces and literal percent characters are supported.

```rust
use shroom_core::{Config, Core, WorkspaceOptions};
use std::path::PathBuf;

async fn example() -> shroom_core::Result<()> {
    let mut core = Core::open(
        PathBuf::from("/path/to/short/state"),
        Config {
            runtime_executable: PathBuf::from("/path/to/msb"),
            firmware: PathBuf::from("/path/to/libkrunfw"),
        },
    ).await?;
    let name = "project".parse()?;
    let workspace = core.create(name, 2222_u32.try_into()?, WorkspaceOptions::default()).await?;
    let connection = workspace.ssh.expect("create authenticated successfully");
    println!("{}@{}", connection.user, connection.endpoint);
    Ok(())
}
```

Creation optionally accepts a guest username and host folders. For example:

```rust
use shroom_core::{FolderAccess, HostFolder, WorkspaceOptions};

let options = WorkspaceOptions {
    user: "arctic".parse()?,
    folders: vec![HostFolder::new(
        "/absolute/path/to/project".into(),
        "/mnt/project".into(),
        FolderAccess::ReadWrite,
    )?],
};
let workspace = core.create(name, 2222_u32.try_into()?, options).await?;
```

These choices are set at creation and persist across restarts. See the
[guest profile](docs/references/shroom-core.md#guest-profile) for account and folder constraints.

Pass the returned identity, host key or `known_hosts` file, and `HostKeyAlias` to your SSH client.
Refresh connection details after each start. The full trust options are defined in the core contract.
Dropping `Core` releases its directory lock and leaves detached workspaces running.

## Agent App Integrations

`shroom-integrations` exports SSH configuration for native agent apps and builds runnable guest commands
and loopback web forwards. Start with fresh core connection details:

```rust
use shroom_core::SshConnection;
use shroom_integrations::Attachment;

fn export(connection: SshConnection) -> shroom_integrations::Result<String> {
    let attachment = Attachment::new("shroom-project".parse()?, connection)?;
    Ok(attachment.ssh_config())
}
```

The caller places the returned stanza in its SSH configuration. Native setup guidance covers Codex,
Claude Desktop, Cursor, and ZCode, with ZCode's own host-key verification still unverified.
Kimi CLI and Kimi/DeepSeek Harness web recipes use tools installed in the guest. Web forwards take the
application's actual reported URL, including any login token, because the requested port may change.
See the [integration contract and examples](docs/references/agent-integrations.md) for process ownership,
connection refresh, and [validation limits](docs/evaluations/2026-09-20-agent-integrations/README.md).

## Lume HTTP SDK

`shroom-lume` calls a separately managed local Lume service through HTTP; it never invokes the Lume CLI.
It supports native VM operations for Linux and macOS. See the
[client contract and required service patch](docs/references/shroom-core.md#lume-http-client) before use.
This is the VM client portion of the integration; the
[shared SSH workspace adapter](docs/proposals/lume-workspaces.md) still needs additional service capabilities.

```rust
use shroom_lume::{Client, Config};
use std::time::Duration;

async fn inspect() -> shroom_lume::Result<()> {
    let client = Client::new(Config {
        address: ([127, 0, 0, 1], 7777).into(),
        storage: "/path/to/dedicated/lume-vms".into(),
        request_timeout: Duration::from_secs(30),
    })?;
    for vm in client.list().await? {
        println!("{}: {:?}", vm.name, vm.state);
    }
    Ok(())
}
```

Creation and start return asynchronous acknowledgements. Linux creation allocates an empty disk;
clone a prepared VM for a bootable guest. macOS installation requires a local IPSW.
Native destructive operations are named `force_stop` and `force_delete` to reflect Lume's behavior.
