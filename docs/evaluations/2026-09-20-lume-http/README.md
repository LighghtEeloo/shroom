# Lume HTTP Client Evaluation — 2026-09-20

This evaluation checks whether Lume's existing HTTP API can support Shroom and verifies the new Rust
client's transport behavior. The implemented contract lives in the
[Lume HTTP client reference](../../references/shroom-core.md#lume-http-client); the remaining workspace integration
lives in the [proposal](../../proposals/lume-workspaces.md).

## Source and Environment

Reviewed Cua revision: `9bbfa7dd3e27ca7f1861ede70aaca390174493f9`.
The source was inspected on macOS ARM64 with the repository's pinned Rust toolchain and Swift 6.4.
The Swift [package manifest](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/Package.swift)
exports an executable and requires macOS 14 or newer. It provides no library product for embedding in Rust.

No Lume executable, running service, or prepared Lume guest image was available during this evaluation.
No Linux or macOS VM was booted through this client. The checks below establish source compatibility and
HTTP behavior, not runtime acceptance or SSH workspace readiness.

## Observed API Behavior

| Source observation | Evidence |
| --- | --- |
| Create and run acknowledge asynchronous work with `202`; deletion succeeds with an empty `200` body | [Handlers.swift](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/src/Server/Handlers.swift) |
| Get/list/delete take storage in the query; stop takes storage in JSON | [Server.swift](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/src/Server/Server.swift) |
| Create accepts `linux`/`macos`, binary sizes spelled `MB`/`GB`, and an IPSW path; run has explicit VNC, clipboard, and sharing options | [Requests.swift](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/src/Server/Requests.swift) |
| Normal inspection includes resource and network fields; active pulls can return only name, status, and progress | [VMDetails.swift](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/src/VM/VMDetails.swift), handlers above |
| Native stop calls `VZVirtualMachine.stop`; process-based fallback can escalate to SIGKILL | [VMVirtualizationService.swift](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/src/Virtualization/VMVirtualizationService.swift), [VM.swift](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/src/VM/VM.swift) |
| The running-VM cache uses names, and delete may stop a running VM before deleting it | [LumeController.swift](https://github.com/trycua/cua/blob/9bbfa7dd3e27ca7f1861ede70aaca390174493f9/libs/lume/src/LumeController.swift) |

The reviewed HTTP routes expose neither guest administration nor graceful shutdown, conditional deletion,
loopback port publishing, or the core's public-only network policy. Native NAT and bridging are available.
The inference for Shroom is that its existing SSH workspace contract cannot be implemented by mapping
those six operation names onto Lume's current routes.

## Rust HTTP Checks

`crates/shroom-lume/tests/http.rs` uses temporary loopback TCP servers and exercises the real HTTP client.
The fixtures follow the reviewed Swift types and routes. The six tests cover:

- Linux and macOS inspection and creation, clone, asynchronous run, stop, and empty-body deletion;
  exact method/path/payload shapes; explicit storage; disabled sharing/VNC/clipboard.
- Valid and invalid service addresses, paths, names, allocations, and IPSW paths; invalid inputs issue no requests.
- Sparse, unknown, and stale native states; original HTTP error messages; malformed responses and identity mismatches.
- Redirect refusal and failed mutations without automatic replay.
- Response-size limits with both content-length and chunked transfer encoding.
- Request deadlines that include stalled response bodies.

Reproduce the workspace checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The test host must allow local TCP listeners. The existing opt-in microsandbox runtime tests remain ignored
in this command; their prior evidence is in the [core evaluation](../2026-09-20-minimal-core/README.md).
Formatting and Clippy passed. All 17 ordinary Rust tests passed, including the six new HTTP tests;
the two existing opt-in runtime tests were ignored. The Swift routing and query checks below also passed.

## Storage Routing Reproducer

The reviewed router compares the complete request target with the route's static components.
`GET /lume/vms?storage=...` therefore fails to match `/lume/vms`. Removing the query only for parameter
extraction is insufficient because route selection happens first.
The checked-in [patch](../../../integrations/lume/http-routing.patch) fixes route matching.

`check-router.py` extracts the real Swift `Route` and query parser from the reviewed source. It compiles
both the original route and a patched copy, reproduces the original failure, and checks six successful
and rejected routing cases. It also demonstrates why query spaces need `%20` and why literal percent
characters are rejected. The source checkout is not modified.

```sh
git clone --filter=blob:none --sparse https://github.com/trycua/cua.git /tmp/cua-lume-review
git -C /tmp/cua-lume-review sparse-checkout set libs/lume
git -C /tmp/cua-lume-review checkout 9bbfa7dd3e27ca7f1861ede70aaca390174493f9
python3 docs/evaluations/2026-09-20-lume-http/check-router.py /tmp/cua-lume-review
```

To apply the verified routing change when building that Lume service:

```sh
git -C /tmp/cua-lume-review apply "$PWD/integrations/lume/http-routing.patch"
```

The patch has not been submitted upstream. It fixes routing only; it does not add the missing workspace
capabilities or qualify the complete Lume service for Shroom's acceptance tests.
