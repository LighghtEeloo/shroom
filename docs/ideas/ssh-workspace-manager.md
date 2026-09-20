# SSH-First Coding Workspaces

## Motivation

Use existing coding-agent apps in isolated, persistent workspaces shared across agents,
without replacing their interfaces.

## Design

Build a lightweight, cross-platform Rust app over microsandbox, with priority on macOS and Linux.
The [Shroom core reference](../references/shroom-core.md#ssh-workspace-core) defines the thin SDK integration
and SSH connection contract.
The [agent integration reference](../references/agent-integrations.md) defines connection exports and launch recipes.

Target native SSH for Codex, Claude Desktop, Cursor, and ZCode.
Run Kimi Code and `dsh` inside the guest through SSH or forwarded web interfaces.
Use per-workspace authorization, persistent host keys, and non-root users; do not share host files
or credentials by default.

## Agent Integration Status

The connection and command layer is implemented in `shroom-integrations`.
The [integration reference](../references/agent-integrations.md#attachments-and-native-applications) owns native
application setup and trust requirements; its [web integration section](../references/agent-integrations.md#web-applications-and-forwarding)
describes Kimi and DeepSeek Harness launch/forwarding. The
[evaluation](../evaluations/2026-09-20-agent-integrations/README.md) distinguishes transport validation from
the remaining application-level compatibility checks.

## Tasks

Build on the [Shroom core](../references/shroom-core.md#ssh-workspace-core).
Broader product work includes packaging pinned runtimes and guest images and validating agent applications.
Validate each agent's guest-bound editing and execution, reconnects, persistence,
and access isolation across supported platforms.

Defer custom agent interfaces, MCP, cloud workers, synchronization, credential management, and conversation migration.
Exclude Grok Bot until a workspace-bound integration is established.
