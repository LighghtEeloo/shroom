# Shroom

Shroom is an SSH-first workspace library over microsandbox. The core exposes `create`, `list`, `get`,
`start`, `stop`, and `remove`, returning connection details for existing SSH clients.
See the [core contract](docs/references/minimal-core.md) and [documentation index](docs/README.md).

The repository contains one Rust crate in `crates/shroom-core`, a prepared guest recipe in
`images/workspace`, and documentation in `docs`. It has no CLI or resident service.

## Development

The pinned Rust toolchain is installed through rustup. Dependencies live in the root workspace manifest.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Normal tests require host `ssh` and `ssh-keygen`, but do not boot VMs. Real-runtime acceptance is opt-in;
see [validation](docs/evaluations/2026-09-20-minimal-core/README.md).

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
with the SDK's `Never` pull policy. Runtime and image installation remain explicit setup steps.
Choose a short state path because the SDK derives Unix socket paths from it. Paths must be UTF-8
without control characters or `$`; spaces and literal percent characters are supported.

```rust
use shroom_core::{Config, Core};
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
    let workspace = core.create(name, 2222_u32.try_into()?).await?;
    let connection = workspace.ssh.expect("create authenticated successfully");
    println!("{}@{}", connection.user, connection.endpoint);
    Ok(())
}
```

Pass the returned identity, host key or `known_hosts` file, and `HostKeyAlias` to your SSH client.
Refresh connection details after each start. The full trust options are defined in the core contract.
Dropping `Core` releases its directory lock and leaves detached workspaces running.
