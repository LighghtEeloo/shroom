# Documentation

References record implemented contracts. Proposals describe remaining feature work, evaluations record
reproducible evidence and platform validation limits, and ideas describe the broader product direction.

- [Shroom core](references/shroom-core.md): the microsandbox SSH workspace contract and the typed Rust
  HTTP client for a local Lume service, including their responsibilities and validation requirements.
- [Desktop app](references/desktop-app.md): the Freya workspace shell, lifecycle and recovery flows,
  SSH verification, native agent setup, managed web connections, connection copying, and runtime setup.
- [Agent app integrations](references/agent-integrations.md): native SSH exports, guest command recipes,
  and loopback web forwarding for existing agent applications.
- [Lume workspaces](proposals/lume-workspaces.md): service capabilities needed for the shared SSH workspace interface.
- [SSH-first coding workspaces](ideas/ssh-workspace-manager.md): product motivation and remaining integration work.
- [Why Shroom works](ideas/why-shroom.md): naming and positioning.
- [Core validation](evaluations/2026-09-20-minimal-core/README.md): runtime acceptance and reproduction commands.
- [Desktop app validation](evaluations/2026-09-20-desktop-app/README.md): Freya UI checks and real-runtime
  missing-image recovery, import, and workspace lifecycle validation.
- [Lume HTTP validation](evaluations/2026-09-20-lume-http/README.md): source review, HTTP tests, and routing regression evidence.
- [Agent integration validation](evaluations/2026-09-20-agent-integrations/README.md): SSH export and forwarding
  coverage, with application compatibility checks still to run.
