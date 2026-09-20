# Documentation

References record implemented contracts. Evaluations record reproducible evidence
and platform validation limits; ideas describe the broader product direction.

- [Minimal core](references/minimal-core.md): a thin headless Rust layer over microsandbox,
  with guest SSH setup, access identity, connection details, and acceptance tests.
- [Agent app integrations](references/agent-integrations.md): native SSH exports, guest command recipes,
  and loopback web forwarding for existing agent applications.
- [SSH-first coding workspaces](ideas/ssh-workspace-manager.md): product motivation and remaining integration work.
- [Why Shroom works](ideas/why-shroom.md): naming and positioning.
- [Core validation](evaluations/2026-09-20-minimal-core/README.md): runtime acceptance and reproduction commands.
- [Agent integration validation](evaluations/2026-09-20-agent-integrations/README.md): SSH export and forwarding
  coverage, with application compatibility checks still to run.
