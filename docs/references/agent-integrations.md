# Agent App Integrations

Shroom lets existing agent applications work inside the same persistent guest. Each application already
owns its interface and agent protocol; the integration layer supplies the SSH connection or launch command
that puts its filesystem operations in the workspace. This keeps application support independent of VM lifecycle.

`shroom-integrations` consumes the core's `SshConnection`. It exports native SSH configuration, builds guest
commands, and describes local web forwards. The [core reference](minimal-core.md) owns SSH
identity, provisioning, and connection refresh. The [integration evaluation](../evaluations/2026-09-20-agent-integrations/README.md)
records transport evidence and the application compatibility checks still to run.

## Attachments and Native Applications

An `Attachment` is an immutable snapshot of a current SSH connection and a caller-selected `SshAlias`.
Get fresh connection details from `Core::get` or `Core::start` before making a new attachment. Reuse the
chosen alias when refreshing an existing workspace, and give workspaces in different state roots distinct aliases.
Constructing an attachment performs input validation without accessing files or checking service health.

```rust
use shroom_core::SshConnection;
use shroom_integrations::{Attachment, NativeApp};

fn export(connection: SshConnection) -> shroom_integrations::Result<String> {
    let attachment = Attachment::new("shroom-project".parse()?, connection)?;
    println!("{}", NativeApp::Codex.setup());
    Ok(attachment.ssh_config())
}
```

`ssh_config()` renders one concrete `Host` stanza for the caller to place in its chosen configuration.
Put the stanza before matching defaults: OpenSSH usually takes the first value for each option.
For Codex discovery, place the concrete entry directly in `~/.ssh/config`; discovery of an entry in an
included file has not been validated. The library never edits that file or app settings itself.
An exported stanza cannot remove additive identities or forwards elsewhere in a user's configuration,
so check the application's effective configuration with `ssh -G <alias>` when incorporating it.

The stanza carries the current endpoint, workspace user, client identity, dedicated known-hosts file,
and endpoint-independent `HostKeyAlias` required by the core's trust contract. It requests strict checking,
public-key authentication, no SSH agent or agent forwarding, and no connection multiplexing. It leaves
remote commands and application forwarding to the native client. An ordinary native SSH session begins
in the user's home; select `connection().directory` as the app's remote project directory.

`connection()` also exposes the real TCP endpoint and original public host key for clients with their own
SSH implementation. `NativeApp` supplies setup guidance, a documentation URL, and `HostKeyHandling`.
The latter distinguishes configuration-based connection from a client whose own host-key handling must
be established; it is not an application certification.

| Application | Documented connection route | Guest setup and remaining check |
| --- | --- | --- |
| [Codex](https://learn.chatgpt.com/docs/remote-connections#connect-to-an-ssh-host) | Concrete SSH alias resolved with OpenSSH | Install and authenticate Codex in the guest; ensure `codex` is on the login-shell `PATH`. Validate a remote app session. |
| [Claude Desktop](https://code.claude.com/docs/en/desktop#ssh-sessions) | SSH-config alias in an SSH connection | Desktop installs Claude Code remotely on first connection. Validate installation and effective trust options. |
| [Cursor](https://cursor.com/docs/agent/agents-window) | Native Remote SSH workflow | Validate its guest server installation, effective trust options, and remote project operations. |
| [ZCode](https://zcode.z.ai/en/docs/remote-development) | Direct endpoint, username, and identity file | Alias import only prefills those fields. Its enforcement of Shroom's public host-key pin remains unverified. |

ZCode is marked `ClientVerificationRequired`. Providing its direct connection fields does not establish
support for `HostKeyAlias` or the dedicated known-hosts file. Verify a supported pinning path in the installed
client before using it, and refresh cached endpoint fields on reconnect. Grok Bot remains deferred because
the product investigation has not established a workspace-bound connection path.

## Guest Commands

`Attachment::command` builds a `std::process::Command` for host OpenSSH. It ignores ambient SSH configuration
and uses the attachment's trust options. The guest command runs through the prepared image's Bash login shell,
changes into the workspace directory, and replaces that shell with the requested program. A missing directory
fails before running the program. Login-shell startup makes user-installed tools available through the guest's
`PATH`; the caller still owns their installation and authentication.

`RemoteCommand` stores an executable and literal arguments. Each is quoted independently for the guest shell;
the host never evaluates a shell command string. Callers that need a script can explicitly invoke `/bin/sh -c`
and supply the script as an argument. `Terminal::Interactive` requests a PTY, while `Terminal::None` supports
captured output and long-running web servers. `Attachment::kimi_command` builds the interactive Kimi CLI command.

```rust,no_run
use shroom_integrations::{Attachment, RemoteCommand, Terminal};

fn check_codex(attachment: &Attachment) -> Result<(), Box<dyn std::error::Error>> {
    let remote = RemoteCommand::new("codex")?.with_arg("--version")?;
    let output = attachment.command(&remote, Terminal::None).output()?;
    if !output.status.success() {
        return Err("Codex is unavailable in the guest login shell".into());
    }
    Ok(())
}
```

## Web Applications and Forwarding

Kimi and DeepSeek Harness share one launch shape: run an installed guest executable with `web --no-open`,
bind it to guest loopback, and supply a requested port. `WebApp::launch_command` builds this command;
`default_port()` returns 5494 for Kimi and 3080 for DeepSeek Harness. Install `kimi` or `dsh` into the guest
login-shell `PATH` beforehand. DeepSeek's documented npm launcher can be used for explicit guest setup,
but Shroom's launch path calls the installed `dsh` executable and performs no package download.
Kimi runs with guest `PYTHONUNBUFFERED=1` so its reported URL is available when the caller captures stdout.

The requested port is not proof of the actual endpoint. [Kimi searches subsequent ports when one is occupied](https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/web/app.py),
including when `--port` is explicit. Start the guest process, read its reported URL, and parse that URL as
`GuestWebUrl` before building a forward. This also preserves any query or fragment used for application login.
Shroom does not parse version-dependent terminal banners or infer readiness from a listening host port.

```rust,no_run
use shroom_core::HostPort;
use shroom_integrations::{Attachment, GuestWebUrl, WebApp, WebTunnel};
use std::process::{Child, Stdio};

fn launch_kimi(attachment: &Attachment) -> std::io::Result<Child> {
    WebApp::Kimi.launch_command(attachment, WebApp::Kimi.default_port())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
}

// Call after reading the actual URL from the running application's output.
fn forward_web(
    attachment: &Attachment,
    reported_url: &str,
    local_port: HostPort,
) -> Result<(Child, WebTunnel), Box<dyn std::error::Error>> {
    let guest: GuestWebUrl = reported_url.parse()?;
    let tunnel = attachment.web_tunnel(guest, local_port);
    let child = tunnel.command().stdin(Stdio::null()).spawn()?;
    Ok((child, tunnel))
}
```

`GuestWebUrl` accepts HTTP URLs with an explicit unprivileged port and a host of `127.0.0.1`, `localhost`,
or `::1`. It rejects credentials, external hosts, and control characters. `web_tunnel` builds an OpenSSH
local forward bound to host `127.0.0.1`. The destination remains guest loopback; `localhost` maps to
guest IPv4 loopback, matching the launch recipes. `local_url()` changes the host and port while retaining
the path, query, and fragment. Treat URLs containing login tokens as credentials when displaying or logging them.

The forwarding command uses `ExitOnForwardFailure=yes`, so an occupied host port fails instead of silently
continuing without a forward. This only establishes SSH forwarding: the caller must check the guest application's
startup and HTTP response before presenting its local URL as ready. The
[DeepSeek web CLI reference](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/cli/reference/README.md#web-profile)
describes its startup and browser behavior. Application authentication remains inside the guest.

## Ownership and Limits

Every launch method returns an unstarted command. The caller owns spawning, draining output, observing exit,
cancelling, and reaping the resulting process. A `std::process::Child` is not killed on drop. Async callers can
convert commands to `tokio::process::Command` and use `kill_on_drop(true)` for the local SSH process.
Stopping a forwarding process closes its local listener; it does not stop a separately launched guest app.
Disconnecting its launch SSH session is not a guarantee that the remote process exits either.

Installers, automatic app registration, credential copying, browser launching, guest process supervision,
and automatic reconnection remain caller responsibilities. VM stop/start ends existing sessions and forwards;
obtain fresh core connection details, restart the required guest app, and create new commands afterward.
The integration layer has no resident process, application protocol, or second workspace catalog.
