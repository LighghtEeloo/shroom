# Agent Integration Validation — 2026-09-20

The [integration contract](../../references/agent-integrations.md) is implemented in `shroom-integrations`.
The library exports connections and commands; desktop app installation and authenticated agent sessions
are separate compatibility checks. This report keeps transport evidence distinct from those checks.

All ten ordinary integration tests passed on macOS ARM64.
The three reference examples compile as Rustdoc tests, and formatting and Clippy with warnings denied pass.
The VM acceptance test passed in 2.18 seconds against microsandbox 0.7.2 and the prepared image used by the
core acceptance run. It also authenticated with copied access paths containing spaces, percent signs,
single/double quotes, and backslashes. The first sandboxed boot was denied by the host execution sandbox;
the passing test ran with virtualization and loopback-listener access.

## Reproduction

Ordinary tests use host OpenSSH to resolve generated configuration and the host shell to exercise literal
argument quoting. Fake `kimi` and `dsh` executables capture recipe arguments without installing agent apps
or making provider requests.

```sh
cargo test -p shroom-integrations --locked
```

The opt-in test uses the same runtime and guest profile as the
[core acceptance run](../2026-09-20-minimal-core/README.md#tested-inputs). It creates a private SDK home under
`/tmp`, imports the supplied image, and boots one disposable workspace. The image contains Perl through Git's
Debian dependencies; a small guest HTTP fixture tests forwarding independently of agent installation.

```sh
SHROOM_TEST_RUNTIME=/path/to/msb \
SHROOM_TEST_FIRMWARE=/path/to/libkrunfw \
SHROOM_TEST_IMAGE_ARCHIVE=/tmp/shroom-workspace.tar \
  cargo test -p shroom-integrations --test runtime --locked \
  -- --ignored --nocapture
```

The test needs virtualization and permission to bind local TCP listeners. It uses host ports 34522–34524;
set `SHROOM_INTEGRATION_TEST_PORT` to change the first port. Success gracefully stops/removes the guest and
deletes test state. Failure prints and retains its private state root for inspection and explicit cleanup.
It never edits the user's SSH config or uses the default microsandbox catalog.

## Coverage

| Boundary | Evidence |
| --- | --- |
| Validation | Concrete aliases and port bounds paired with rejected patterns, options, control characters, malformed paths, and nonlocal URLs |
| Connection export | OpenSSH resolves the host/user/port, a single identity, and the expected strict host-key options |
| Shell boundary | Spaces, quotes, substitution syntax, percent signs, empty arguments, and newlines remain literal; missing directories prevent execution |
| App recipes | Installed executable names, guest directory, loopback bind, requested port, disabled browser opening, Kimi unbuffered output, and PTY selection |
| URL translation | The reported port is used; path, query token, and fragment survive local-port mapping; external URLs are rejected |
| Real guest | Non-root execution and config-based login, including access paths with spaces, percent signs, quotes, and backslashes |
| Rejected authentication | An unrelated client identity and a deliberately wrong host pin fail; original access material remains unchanged |
| Real forward | A host HTTP request reaches a guest loopback service through the generated OpenSSH forward |
| Port conflict | A second forward on an occupied port exits unsuccessfully and preserves the running service |
| Persistence | Files, client identity, and host pin survive graceful stop/start and reopening `Core` |

## Source Review and Remaining Compatibility Checks

Reviewed on 2026-09-20: the official app documentation linked from the integration reference,
[Kimi web startup](https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/web/app.py), and
[DeepSeek web argument parsing](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/bundle/web-app/src/startup.ts).
Kimi's port fallback motivates taking the actual reported URL. DeepSeek's web flags include `--host`,
`--port`, and `--no-open`; its browser trust check accepts loopback authorities with the forwarded port.

Run authenticated remote sessions in Codex, Claude Desktop, and Cursor, including edits, terminal operations,
and reconnects. Establish ZCode's explicit host-key pinning path before marking it compatible. Run the actual
Kimi and DeepSeek web applications with the installed guest versions and their normal login flows.
The transport tests do not certify those applications. Linux x86-64 VM acceptance and a direct OCI build of the
guest Dockerfile remain unrun on this macOS ARM64 development host.

## Codex Project Handoff

The installed desktop app at `/Applications/ChatGPT.app`, version **26.915.31945**, was inspected read-only
on 2026-09-20. Its `Info.plist` registers the `codex` URL scheme. In `Contents/Resources/app.asar`,
`.vite/build/bootstrap-DF0QwAxC.js` recognizes `settings/connections/ssh/add`, accepting `name` (or `alias`),
`projectPath`, and `enabled`. The parser trims the folder parameter and permits only the expected query keys.

The renderer in `webview/assets/remote-connections-settings-b8d26c3afb7b.js` handles this route by calling
SSH connection registration, creating or reusing a project matching the host and absolute remote path,
selecting the project, and enabling the connection when requested. This establishes a concrete handoff
mechanism in the installed build; it is not a stability guarantee for other desktop versions.
The [official guide](https://learn.chatgpt.com/docs/remote-connections#connect-to-an-ssh-host)
documents native SSH projects and guest CLI requirements but does not document this route.
The installed `codex app --help` exposes a local workspace path, with no SSH-host option.

Shroom's regression tests exercise URL encoding for Unicode, spaces, query/fragment delimiters, quotes,
and percent signs, paired with rejection of a directory that Codex would trim. App tests use temporary SSH
configuration and real `ssh -G` resolution to cover installation, repeat registration, endpoint refresh,
unrelated-host preservation, private backups, file modes, rejected inherited identities/commands/forwards,
malformed config, damaged ownership markers, alias conflicts, concurrent edits, lock release, and symlinks.
The UI test verifies that **Add to Codex** replaces the existing primary copy action only for Codex.
The app's 40 ordinary tests, full workspace tests, Clippy with warnings denied, formatting, and native build
pass. The full workspace run needed loopback permission for the existing Lume HTTP fixture listeners;
the first sandboxed run could not bind those listeners.

```sh
cargo test -p shroom-integrations -p shroom-app --locked
```

These checks do not register a project in the user's desktop app or edit their SSH config. A live authenticated
Codex guest session, including the app's consent and login flows, remains an explicit compatibility check.

### Missing Guest CLI Regression

A live **Add to Codex** attempt exposed a missing prerequisite: the handoff opened the desktop app without
installing the guest CLI. Guest preparation now follows the
[desktop contract](../../references/desktop-app.md#agent-connections) before registration or URL dispatch.
Seven new regression tests cover a missing CLI, reuse of existing installations, preservation of a broken CLI,
failed downloads and installers, missing output binaries, login `PATH` failures, bounded diagnostics, process
timeouts, and the attachment's pinned SSH command. All 47 ordinary app tests, ten integration tests, and three
Rustdoc examples passed; Clippy with warnings denied and formatting also passed.

The failure was reproduced in the existing Debian ARM64 `zydeco` guest on 2026-09-20: SSH authenticated as
`developer`, while the login shell had no `codex` command. Running the new setup script installed the official
standalone CLI **0.155.1** without root or Node.js. A fresh SSH login then resolved
`/home/developer/.local/bin/codex` and successfully ran `codex --version`. `codex login status` reported
`Not logged in`; authenticated app operation remains unverified and requires the user's sign-in.
