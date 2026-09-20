# Guest Sudo Validation — 2026-09-20

The [guest profile](../../references/shroom-core.md#guest-profile) grants passwordless sudo to the
selected workspace account. Real-runtime acceptance passed on macOS ARM64 with microsandbox 0.7.2.
The complete core runtime sequence passed in 34.41 seconds; the subsequent custom-account run with an
additional elevated read-only mount check passed in 4.61 seconds.

## Observed Coverage

- Default `developer` and custom `arctic` SSH sessions retain UID 1000 and run `sudo -k -n id -u` as UID 0,
  both immediately after creation and after stop/start.
- `visudo -c` accepts the complete guest configuration. The account-specific grant is root-owned with
  mode `0440`; an unprivileged attempt to append to it fails and leaves its contents unchanged.
- Direct root SSH login fails. Ordinary shell and SFTP writes to a root-owned test file still fail.
- Guest writes to a read-only host folder fail with a read-only error, including through sudo;
  the host sentinel retains its original contents.
- An existing account-name collision leaves the developer account, SSH identities, and sudo grant untouched.
- Removing either `sudo` or `visudo` from an unprovisioned test guest causes provisioning to exit with
  code 2 and the rebuild/reimport diagnostic, before renaming the account or installing access files.

## Reproduction and Limits

Build an image archive using the [repository setup instructions](../../../README.md), then run:

```sh
SHROOM_TEST_RUNTIME=/path/to/msb \
SHROOM_TEST_FIRMWARE=/path/to/libkrunfw \
SHROOM_TEST_IMAGE_ARCHIVE=/path/to/workspace.tar \
  cargo test -p shroom-core --lib core::runtime_tests --locked \
  -- --ignored --nocapture --test-threads=1
```

The test imports into temporary private catalogs and removes its guests on success.
No OCI build engine was running on this host. The test archive was assembled by importing the
[previous prepared Debian image](../2026-09-20-minimal-core/README.md#tested-inputs) into a disposable VM,
installing `sudo` with `apt-get install -y --no-install-recommends`, and exporting the unprovisioned
filesystem without runtime mounts or generated network identity files. The core installed the current
checked-in SSH helper during creation. The exported image contains no SSH host keys, authorized client
keys, or workspace sudo grant. The image-building VM was stopped and removed after testing.

The tested image archive's SHA-256 is
`e4fe7a63c0bf7bb7b887357c3c7e9a0aba210ff0cccc7be5941b37bf2d5eb513`.
Direct Dockerfile builds and Linux x86-64 runtime acceptance remain unrun.

Workspace tests, formatting, Clippy with warnings denied, and the helper's shell syntax check passed.
The first restricted workspace test run could not bind the Lume HTTP test servers; rerunning with
local socket access passed the full suite.
