# Lume Workspaces

The intended feature is Linux and macOS SSH workspaces backed by Lume, using the same workspace operations
as microsandbox. The implemented [HTTP client](../references/shroom-core.md#lume-http-client) handles Lume's native
VM lifecycle. It cannot yet supply several capabilities required by the
[core contract](../references/shroom-core.md#ssh-workspace-core), so no shared workspace trait has been introduced.

This proposal covers that remaining integration. It is not an implemented or approved replacement for
the existing SSH and isolation guarantees. The [source evaluation](../evaluations/2026-09-20-lume-http/README.md)
records the upstream evidence behind the capability gaps.

## Capabilities Required From Lume

The simplest shared interface is at the workspace boundary: creation and start return verified SSH access,
inspection reports state, stop requests graceful shutdown, and removal requires quiescence.
Runtime mechanisms can differ while callers retain those meanings. The following upstream additions would
allow Lume to meet that boundary without moving VM management into Shroom.

| Required capability | Service change and acceptance evidence |
| --- | --- |
| Trusted guest bootstrap | Supply a guest administration channel for Linux and macOS; install the client's public key, generate a unique guest host key, and return its public half through that trusted channel |
| Restricted guest networking | Publish guest SSH on host loopback and enforce the public-only egress policy; test denied host/private-network access as well as allowed public access |
| Graceful shutdown | Add a request that asks the guest to shut down and reports a timeout without escalating to forced termination |
| Quiescent removal | Atomically reject deletion when a runtime still exists; an earlier inspection followed by unconditional delete cannot establish this guarantee |
| Storage-qualified identity | Key running VMs and asynchronous operations by storage plus native identity, including clone, inspection, stop, and removal |

A cloned template alone does not establish a fresh SSH identity. Client-side key scanning cannot replace
the trusted bootstrap channel, and password-based guest SSH is a different provisioning contract.
Likewise, a client-side status check cannot make Lume's destructive delete conditional on the same VM
remaining stopped. These operations require service support rather than extra HTTP wrapper methods.

## Shared Interface After Service Support

Once Lume can pass the workspace acceptance tests, extract the existing access handling and workspace
domain types into the common core. Keep `Core` as the caller-facing workspace API and select the concrete
runtime through typed configuration. An internal runtime interface should describe the operations both
adapters actually implement, including trusted guest administration and current SSH endpoint discovery.
Keep native error sources beneath operation context.

Replace the microsandbox-only status field with a workspace state type only when both adapters exist.
Map native states conservatively: an unfamiliar state cannot imply readiness or quiescence.
Keep operating system selection in the creation profile; guest user and working-directory defaults differ
between Linux and macOS. The runtime continues to own VM identifiers, disks, lifecycle, and detached execution.

Implement the refactor with the Lume adapter and update the existing callers in the same change.
Do not leave a second catalog, name-to-ID mapping, or compatibility layer alongside the native runtimes.
Run the existing SSH trust, persistence, caller-lifetime, shutdown, cleanup, and isolation acceptance tests
against microsandbox, Lume Linux, and Lume macOS.

## Decision Needed

Preserving the current workspace contract requires extending Lume's service capabilities above.
That increases the upstream work beyond using its existing HTTP API.
An alternative is to approve a separate Lume workspace profile with caller-prepared SSH access, native NAT
networking, and explicitly forceful lifecycle operations. Such a profile needs its own documented behavior
and cannot silently implement the current `Core::stop` and `Core::remove` promises.

The core reference says to revisit the runtime choice or scope when a gap would require Shroom to own
supervision, persistence, or networking. The remaining decision is therefore whether to extend Lume while
preserving that contract, or explicitly approve a different Lume workspace profile. The HTTP client is useful
under either choice and does not settle it implicitly.
