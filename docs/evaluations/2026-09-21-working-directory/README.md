# Working Directory Validation — 2026-09-21

The editable [default working directory](../../references/desktop-app.md#default-working-directory) passed
unit, process, Freya UI, and real-guest acceptance checks on macOS ARM64 with microsandbox 0.7.2. The
[project integration contract](../../references/agent-integrations.md#projects) owns path and launch behavior;
this report records the observed results and their limits.

## Observed Coverage

The final workspace suite passed 92 tests, including four documentation tests, with nine opt-in tests skipped
by that command. Four real-guest tests and the screenshot renderer were run separately as described below.
Formatting, Clippy with warnings denied, the native app build, and local documentation links also passed.

The dedicated guest acceptance test passed in 6.44 seconds using the custom account `arctic` and a read-only
host-folder mount. It saved a folder containing spaces and a quote, checked it over pinned SSH, and executed
the copied terminal command through a host shell with a real guest PTY. Interactive Bash reported the selected
directory. Saving while stopped, reopening the app's state root, and starting again preserved the override.
Removal and recreation under the same workspace name restored the profile default.

Missing paths, regular files, an inaccessible `/root`, and broken symlinks failed with the directory error
before the requested program created its marker. A read-only mount passed the accessibility check and its
host sentinel remained unchanged. A corrupt preference remained a local error while SSH verification and
home-login export worked; resetting removed the bad record. Fixture Codex executions established that an
initial directory failure prevented preparation, and removing a directory during CLI preparation caused the
second check to reject the handoff. A usable directory completed preparation successfully.

The managed web acceptance test passed in 4.28 seconds. It launched the fixture Kimi executable in
`/home/developer/workspace`, changed the default to `/home/developer`, and refreshed. The original session kept
its generation, directory, output, and reported URL; forwarding still reached its HTTP server. A subsequent
DeepSeek Harness fixture launched in the new directory. Duplicate launch rejection, occupied-port recovery,
close, stop/start, and state-root disconnect also passed. These fixtures validate Shroom's commands and process
ownership without installing real agents or making authenticated provider requests.

The existing integration runtime test passed in 2.66 seconds, covering SSH exports, guest command execution,
and loopback HTTP forwarding through the updated `Project` API. The desktop missing-image recovery and
lifecycle test passed in 3.19 seconds, including the copied home-login command. Successful runs removed their
disposable guests and state roots.

Ordinary tests cover literal path parsing and URL/shell encoding, accepted symlinks, rejection after removing
a previously checked directory, bounded diagnostics, timeouts, private atomic preference writes, root isolation,
reset, malformed and unsupported records, oversized saves, and symlink storage rejection. Rejected saves keep
the prior record. Model tests preserve SSH proof and transport exports while expiring only project exports;
session reconciliation still rejects changed endpoints. Freya tests retain invalid drafts, cancel edits,
and keep home login and active browser actions available when preferences are corrupt.

Headless renders at 1120 × 800 were inspected for stopped-workspace editing and an active web session whose
launch directory differs from the next-launch default. The editor's field and actions were visible in Details,
and the agent view displayed both directories with the existing browser controls.

## Reproduction

Build a prepared archive using the [repository setup](../../../README.md#local-runtime-and-image), then set:

```sh
export SHROOM_TEST_RUNTIME=/path/to/msb
export SHROOM_TEST_FIRMWARE=/path/to/libkrunfw
export SHROOM_TEST_IMAGE_ARCHIVE=/path/to/workspace.tar
```

Run the affected acceptance tests sequentially:

```sh
cargo test -p shroom-app --locked directory_preferences_persist_and_validate_in_a_real_guest \
  -- --ignored --nocapture
cargo test -p shroom-app --locked managed_web_launch_forward_and_shutdown_with_a_real_guest \
  -- --ignored --nocapture
cargo test -p shroom-app --locked model::tests::missing_image_recovery_and_workspace_lifecycle \
  -- --ignored --exact --nocapture
cargo test -p shroom-integrations --test runtime --locked -- --ignored --nocapture
```

The directory test uses host SSH port 35182; `SHROOM_DIRECTORY_TEST_PORT` overrides it. The other tests retain
their documented port overrides. Each prints its temporary state root and preserves it on failure for scoped
inspection and cleanup. Test execution requires local sockets and permission to boot the runtime.

Generate the inspected UI states with:

```sh
mkdir -p /tmp/shroom-directory-preview
SHROOM_PREVIEW_DIR=/tmp/shroom-directory-preview \
  cargo test -p shroom-app --locked render_preview -- --ignored
```

Inspect `working-directory.png` and `agents-ready.png`. The ordinary workspace suite, formatting, and Clippy
with warnings denied are run using the commands in the repository's [development instructions](../../../README.md#development).

## Tested Inputs and Limits

The runtime was `/opt/homebrew/opt/microsandbox/libexec/msb`, with firmware
`/opt/homebrew/opt/microsandbox/libexec/libkrunfw.5.dylib`. The prepared ARM64 archive was the exported image
described in the [guest sudo evaluation](../2026-09-20-guest-sudo/README.md#reproduction-and-limits), with SHA-256
`e4fe7a63c0bf7bb7b887357c3c7e9a0aba210ff0cccc7be5941b37bf2d5eb513`.

Initial large async test frames exceeded the test thread's stack; the affected acceptance harnesses now box
backend request futures. A restricted first VM run could not access virtualization. Reruns with runtime access
passed, and failed-run fixtures were removed after confirming the VM was no longer running.

Linux runtime acceptance, native-window interaction, and live project opening in external desktop clients
remain unrun. Codex preparation used a fixture CLI; no host SSH registration, OS handoff, or provider login was
performed by that test. The existing handoff mechanism retains its documented external-client limits.
