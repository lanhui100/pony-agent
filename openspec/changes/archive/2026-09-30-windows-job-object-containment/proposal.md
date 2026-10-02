# Windows Job Object containment (PA-077)

## Why

ProcessManager currently kills only the direct child. Windows descendants can outlive cancellation, session shutdown or manager disposal. PA-076 explicitly deferred Job Object containment; PA-077 implements that lifecycle protection without changing sandbox approval.

## What Changes

- Windows-only RAII Job Object per managed process, kill-on-close and no breakaway flags.
- Immediately assign the spawned child using Child::as_raw_handle (not PID reopening).
- Fail start on Job setup/assignment failure; clean up the child and never publish a handle.
- Terminate descendants on kill, shutdown and last-owner disposal, including after direct-child exit.
- Preserve non-Windows behavior and autonomous Run fail-closed behavior.
- Explicitly document the post-spawn assignment race; this is best-effort containment, not a sandbox or escape-proof boundary.

## Capabilities

### New Capabilities

- `windows-process-containment`: Windows process-tree lifecycle containment.

### Modified Capabilities

None. The public ProcessBackend request/result types remain unchanged.

## Impact

Core process manager, Windows-specific windows-sys direct dependency, lifecycle tests and architecture documentation. Tracked by `management/task-system/03_TASKS/PA-077-windows-job-object-containment.md`; ADR 0018 records alternatives. No frontend behavior or approval-policy migration.
