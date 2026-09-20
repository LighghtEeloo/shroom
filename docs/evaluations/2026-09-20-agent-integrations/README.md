# Agent Integration Validation — 2026-09-20

The [integration contract](../../references/agent-integrations.md) is implemented in `shroom-integrations`.
The library exports connections and commands; desktop app installation and authenticated agent sessions
are separate compatibility checks. This report keeps transport evidence distinct from those checks.

All eight ordinary integration tests passed on macOS ARM64.
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
