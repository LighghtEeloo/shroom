# Minimal Core Validation — 2026-09-20

The [core contract](../../references/minimal-core.md) is implemented in one library.
The full opt-in runtime sequence passed on macOS ARM64 in 29.41 seconds, including the separate caller process.
Linux x86-64 runtime acceptance remains unrun because this development environment has no such host.
The CI workflow runs formatting, Clippy, and the ordinary tests on Linux and macOS; it does not boot VMs.

Final local checks passed: `cargo fmt --all -- --check`, Clippy with warnings denied and `large_futures`
enabled, all 11 ordinary tests, the runtime suite, the guest helper's shell syntax check, and `git diff --check`.
This report consolidates the temporary development log under the repository's documentation rules.

## Tested Inputs

| Component | Selection |
| --- | --- |
| Rust | 1.98.1, `aarch64-apple-darwin` |
| SDK | `e9565401dacde7e5c3a8fb935574c785aa1bc8f1`, version 0.7.2; `local` and `net` features only |
| Runtime | Homebrew microsandbox 0.7.2, actual `libexec/msb` executable |
| Firmware | The same installation's `libexec/libkrunfw.5.dylib` |
| Guest | Debian bookworm ARM64 with the packages/account and fixed SSH files from `images/workspace` |
| Clients | Host OpenSSH `ssh` and `sftp`, including native local forwarding |

SHA-256 of the tested runtime executable:
`b7292a901ffc93f84860264c6e0d4636cc9aeeddfb76c952f7c2204ea417ae4b`.
SHA-256 of its firmware:
`873ca79191a7a2bc851ad4eea7b29e3ec2e3ab99ef9ed7e399e2eb091e134b07`.

No OCI build engine was running on this host. The local test image was assembled by applying the recipe's
package/account operations inside a disposable Debian microsandbox, exporting its unprovisioned rootfs,
and packaging it as a Docker-compatible image archive. Host keys and authorized client keys were absent.
The corrected repository SSH helper was included before import. This exercises the guest profile and SDK
image path; executing the Dockerfile with an OCI builder remains a separate reproducibility check.
The temporary archive's SHA-256 was
`5be17dd3a03d1cf8d805ef824f786b7775c835ade3fc1a1012f3e6c87d99017e`.

## Reproduction

Build an image archive using the [setup instructions](../../../README.md), then select the actual runtime pair:

```sh
SHROOM_TEST_RUNTIME=/path/to/msb \
SHROOM_TEST_FIRMWARE=/path/to/libkrunfw \
SHROOM_TEST_IMAGE_ARCHIVE=/tmp/shroom-workspace.tar \
  cargo test -p shroom-core --lib core::runtime_tests::runtime_contract \
  --locked -- --ignored --exact --nocapture
```

The test creates a private, short state directory under `/tmp` and imports the archive there.
It creates two resident guests and creates/stops additional guests sequentially to cross a catalog page.
It requires public HTTPS access and free ports starting at 34222 through 34250.
Override the base with `SHROOM_TEST_PORT`; the test validates room for the range.
All test workspaces are gracefully stopped and removed on success.
On failure, the printed root and any detached VMs remain for inspection and explicit cleanup.
No test uses the user's default SDK catalog or prunes a shared image cache.

## Observed Coverage

| Boundary | Passing evidence |
| --- | --- |
| Inputs and ownership | Valid/rejected slugs and ports; exclusive root lock; lock released after drop and failed open |
| Prerequisites and configuration | Unexpected executable rejected before catalog initialization; conflicting profile settings rejected |
| Creation | Independent client/host identities; duplicate name and recorded port preserve existing artifacts |
| External port conflict | Occupied published port fails bounded readiness; surviving VM requires explicit stop before removal |
| Trust | A's key cannot enter B; stale endpoint and deliberately wrong pin fail without changing stored trust |
| Caller lifetime | Separate process creates alpha, exits, then a reopened core and SSH client reconnect |
| Running start | Disabling the guest helper does not affect start of an already running VM; no relaunch occurs |
| Guest access | Developer shell/SFTP writes succeed in its workspace and fail in a root-owned location; root login fails |
| Authentication | The guest offers only public-key authentication; host environment and client private identity are absent |
| Persistence | User file, executable, original client identity, and host key survive graceful stop/start |
| Incomplete access | Missing public key or host pin prevents start and is never regenerated; catalog entry remains visible |
| Endpoint refresh | A second endpoint through an external OpenSSH tunnel authenticates with the original pin |
| Shutdown and removal | Idempotent stop/remove; zero-budget SDK stop preserves VM and access; active/paused removal rejected |
| Isolation | Public HTTPS works; a live host TCP listener and private destination are unreachable from the guest |
| Publishing | Published SSH accepts loopback access and rejects connections to the host's non-loopback address |
| Pagination | Core listing returns all 21 real native records across the SDK's twenty-record default page |
| Helpers | Cancelled host helper exits; output/diagnostics are bounded; SSH ignores ambient configuration and agents |

## Findings Retained in the Implementation

The guest helper must disconnect standard input, output, and error when launching daemonizing sshd.
Ending it with a direct `exec sshd` left one administration caller waiting even though SSH was reachable.
The fixed helper returns promptly on both provisioning and later boots; neither operation needs a supervisor.

OpenSSH path options need quoting and percent escaping. Its `-i` argument performs an initial filesystem check
before later expansion, making a literal percent unreliable through that form. The core uses an explicit
`IdentityFile` option alongside `UserKnownHostsFile`; the runtime fixture retains spaces and a percent in its root.

Concurrent fork/exec can temporarily inherit a lock's file description. An explicit unlock in the state-lock
guard's destructor releases ownership promptly. Native SDK configuration futures also occupy substantial space;
boxing at the existing backend scope boundary fixed a stack overflow in the long acceptance sequence.
The same sequence and Clippy's `large_futures` check verify that fix without changing runtime ownership.

An occupied published port does not necessarily abort this runtime's VM boot. The authenticated probe is therefore
the completion boundary for Shroom creation. The rejected operation leaves the VM and access files available
for explicit inspection, graceful stop, and removal, as the contract requires.

## Remaining Validation

Run the same suite on Linux x86-64 with the matching 0.7.2 runtime pair and a `linux/amd64` image.
Build both architecture images directly from the Dockerfile. The record above does not claim those runs,
host-reboot testing, or compatibility with other SSH client applications.
