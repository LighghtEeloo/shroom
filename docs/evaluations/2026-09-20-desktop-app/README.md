# Desktop App Validation — 2026-09-20

The redesigned Freya app passed its 24 ordinary tests on macOS ARM64, along with the full workspace tests,
Clippy with `-D warnings`, formatting, and a native executable build. Headless Freya renders were inspected
for setup, running workspaces in light and dark appearances, compact navigation at 820 × 640, creation,
environment settings, stopped workspaces, and incomplete-setup recovery.

## Workspace Shell Redesign

Freya interaction tests cover connection-format switching, the Details toggle, compact sidebar navigation,
system appearance changes without losing selection, and form values surviving validation and navigation.
The icon controls also pass keyboard activation, named-tooltip, accessibility-state, and disabled-action tests.
Headless renders confirm Lucide icons from Freya's official collection in both appearances and after navigation;
captures allow the SVG renderer's inherited-color update to complete before recording the painted frame.
The existing deletion, cleanup, disabled-action, and result-delivery regressions also pass in the new shell.
Failed creation selects recovery when the requested workspace or access files exist, while failures without
artifacts retain the creation form; unrelated workspaces do not trigger that navigation.

Model tests verify that discovery and copying do not establish SSH readiness. Successful create, start, and
verification record proof and a timestamp; a failed check records failure. Endpoint changes, stopped states,
catalog failure, and directory changes invalidate proof and connection exports. A failed cleanup preserves
its incomplete selection and activity diagnostics.

The integration test evaluates the copied command with `ssh -G` and compares its effective trust settings
with the existing command builder. Paths include spaces, quotes, backslashes, and percent characters.
The runtime acceptance test then executes the exported command against a disposable workspace and obtains
`developer` from `id -un`, confirming that the command can authenticate without an installed SSH config entry.
It also verifies a running workspace and rejects verification of a stopped one without starting it.

The expanded lifecycle test passed in 3.24 seconds; cleanup through the redesigned Freya controls passed in
0.24 seconds. Both used isolated temporary roots, removed their fixtures, and left existing workspaces alone.
The approved redesign is now recorded in the [desktop app reference](../../references/desktop-app.md).

## Missing-Image Regression

The reported `CreateSandbox: image not cached` failure occurred after the core generated access files,
leaving an incomplete setup absent from the native sandbox catalog. The original GUI showed no way to
import an image or remove that incomplete setup. The app now checks the image before creation, supports
explicit archive import, and lists access directories without native records for confirmed cleanup.
The [desktop app reference](../../references/desktop-app.md#image-setup-and-recovery) owns this behavior.

The opt-in `model::tests::missing_image_recovery_and_workspace_lifecycle` test passed against microsandbox
0.7.2 in 3.01 seconds. It used the runtime, firmware, and prepared ARM64 archive recorded in the
[core evaluation](../2026-09-20-minimal-core/README.md#tested-inputs).
The archive checksum was rechecked as
`5be17dd3a03d1cf8d805ef824f786b7775c835ade3fc1a1012f3e6c87d99017e`.

The test demonstrated that missing-image rejection leaves no new access directory; reproduced the previous
failure through the core; discovered and removed the orphaned access files; rejected an invalid archive;
and imported the prepared archive. It then created a workspace, rejected incomplete-setup cleanup for that
existing workspace, exported its pinned SSH configuration, stopped and restarted it with the same host key,
and removed the disposable workspace. Its temporary state directory was deleted after disconnecting.

## Cleanup Confirmation Regression

The reported stall at **Removing incomplete setup…** came from the lifetime of the Freya task awaiting the
worker. Confirming cleanup removes the confirmation button, which also canceled the task owned by that
button. The interface never received a result and remained busy. The same path affected workspace deletion.
Requests now await their results in the app root's scope.

The retained `ui::tests::confirmed_operations_deliver_errors_after_the_confirmation_unmounts` regression
clicks both kinds of confirmation and checks that a disconnected worker's error reaches the interface.
It failed before the fix by timing out after three seconds and passed afterward for both controls.

The opt-in `ui::tests::confirmed_cleanup_completes_through_the_ui_with_a_real_core` test passed in 0.19 seconds
using microsandbox 0.7.2 and the pinned runtime and firmware. It opened a temporary state directory, created
an incomplete setup fixture, clicked cleanup and confirmation in Freya, and verified that both the files
and the busy indicator disappeared. Refresh and disconnect completed afterward, and the core's directory
lock was released. This test did not boot a VM.

## Scope and Reproduction

The image-recovery runtime test calls the same worker commands as the GUI. Headless Freya tests cover input,
rendering, and cleanup with a real core; OS-level mouse and keyboard interaction was not automated.
Linux runtime and native-window behavior remain unverified in this environment. Run the commands in the
[app validation instructions](../../references/desktop-app.md#validation) to reproduce the tests and screenshots.
