# SSH-First Coding Workspaces

## Motivation

Use existing coding-agent apps in isolated, persistent workspaces shared across agents, without replacing their interfaces.

## Design

Build a lightweight, cross-platform Rust app over the microsandbox SDK, with priority on macOS and Linux. A workspace is a persistent Linux VM with a stable loopback SSH endpoint. An attachment is exported connection details or a launch recipe.

Target native SSH for Codex, Claude Desktop, Cursor, and ZCode. Run Kimi Code and `dsh` inside the guest through SSH or forwarded web interfaces. Use per-workspace authorization, persistent host keys, and non-root users; do not share host files or credentials by default.

## Agents and Integration Analysis

Planned integrations; compatibility with microsandbox remains to be tested.

| Agent | Connection | Integration work and constraints |
| --- | --- | --- |
| [Codex (ChatGPT desktop)](https://learn.chatgpt.com/docs/remote-connections) | Native SSH | Export SSH configuration; install and authenticate Codex in the guest; verify its login-shell `PATH`. |
| [Claude Desktop (Code)](https://code.claude.com/docs/en/desktop#ssh-sessions) | Native SSH | Export connection details. Desktop installs Claude Code remotely on first connection. |
| [Cursor](https://cursor.com/docs/agent/agents-window) | Native remote SSH | Export SSH configuration; validate remote-server installation and operation. |
| [ZCode](https://zcode.z.ai/en/docs/remote-development) | Native SSH | Export a real TCP endpoint and key path; its client ignores `ProxyCommand` and `ProxyJump`. |
| [DeepSeek Harness (`dsh`)](https://github.com/deepseek-ai/deepseek-harness) | Guest web server through an SSH tunnel | Launch `npx @deepseek-ai/dsh web --no-open`; forward its port and open the local URL. |
| [Kimi Code](https://raw.githubusercontent.com/MoonshotAI/kimi-cli/main/docs/en/reference/kimi-web.md) | Guest CLI or tunneled web interface | Launch `kimi` or `kimi web --no-open`; reuse the same tunnel helper for web access. |
| [Grok Bot](https://docs.x.ai/grok-bot/computer-and-apps) | No verified workspace-bound path | Its documented workspace is cloud-hosted. Defer until workspace replacement is established. |

## Tasks

Package a pinned runtime and guest images. Implement workspace lifecycle, SSH serving, connection export, and generic command-and-tunnel recipes. Validate each agent’s guest-bound editing and execution, reconnects, persistence, and access isolation across supported platforms.

Defer custom agent interfaces, MCP, cloud workers, synchronization, credential management, and conversation migration. Exclude Grok Bot until a workspace-bound integration is established.
