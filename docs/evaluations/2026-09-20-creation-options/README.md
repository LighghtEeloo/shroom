# Creation Options Validation — 2026-09-20

The [guest profile](../../references/shroom-core.md#guest-profile) now supports creation-time account and folder
choices. The real VM acceptance test passed on macOS ARM64 with microsandbox 0.7.2 and the prepared Debian
workspace archive. Linux x86-64 runtime validation remains outstanding.

The acceptance run verified a custom `arctic` account with UID 1000, its home and SSH connection metadata,
and rejection of login as the former `developer` account. A writable host directory accepted guest-created files;
a read-only directory allowed reads and rejected writes and deletion. Stop/start and reopening the core preserved
the username, mount settings, files, and SSH host key. Removing the workspace preserved both host directories.

Failure checks rejected overlapping destinations before creating VM or access artifacts. A missing host source
left the workspace stopped and did not prevent catalog inspection. Selecting the image's existing `daemon` account
failed provisioning before installing SSH credentials, leaving the original developer account intact.

The full workspace test suite, the existing core lifecycle acceptance test, Clippy with warnings denied,
the guest helper's shell syntax check, and `git diff --check` passed. UI tests exercised adding multiple folders,
toggling access, removal and replacement, and form validation. A rendered creation screen was visually inspected.
The sandboxed test runner initially denied hypervisor and loopback-listener access; the relevant tests passed
with those host facilities enabled. The failed-test VM directories were removed after verification.

Reproduce the new runtime acceptance test with an installed runtime pair and prepared image archive:

```sh
SHROOM_TEST_RUNTIME=/path/to/msb \
SHROOM_TEST_FIRMWARE=/path/to/libkrunfw \
SHROOM_TEST_IMAGE_ARCHIVE=/path/to/workspace.tar \
SHROOM_TEST_PORT=38522 \
  cargo test -p shroom-core custom_account_and_shared_folders -- --ignored --nocapture
```

The test creates its own short state directory and host folders under `/tmp`, imports the image into its private
catalog, and removes its resources on success. It does not use an existing workspace or share a personal directory.
On assertion failure it retains artifacts for diagnosis, following the existing runtime fixture convention.
