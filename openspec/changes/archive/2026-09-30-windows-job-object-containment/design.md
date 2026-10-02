# PA-077 Design

## Context And Goals

ProcessManager owns direct Child handles and session-scoped process records. SandboxBackend independently authorizes isolated Run execution. Add Windows descendant lifecycle cleanup without granting sandbox availability. Preserve the ProcessBackend public contract and non-Windows implementation.

## Decisions And Alternatives

1. Windows-only private Job RAII wrapper using windows-sys 0.61.2 Foundation, Security and System_JobObjects features. Create an unnamed, non-inheritable Job before spawn, set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, omit both breakaway flags. Use Child::as_raw_handle to assign/verify membership, avoiding PID reuse and unnecessary OpenProcess privileges. Setup failure prevents spawn; assignment failure kills/reaps the child and returns an error before publishing a process handle.
2. Retain std::process::Command for environment/argument/pipe behavior. Immediate post-spawn assignment is best-effort: an early descendant can escape before assignment. The window is scheduler-dependent and has no fixed time upper bound; do not describe it as a bounded millisecond race. Do not promise escape-proof containment. Suspended/raw CreateProcessW or atomic job-list startup is a stronger alternative but changes the process-launch implementation; reviewers must challenge whether the accepted limited guarantee is sufficient.
3. ManagedProcess owns the Job. Explicit kill/shutdown terminates the Job even when the direct child is already exited; repeated terminal cleanup is safe. Job drop closes the handle and kills remaining members when the final manager/entry owner disappears. Retain direct Child reap and final poll state. Natural parent exit does not immediately kill descendants: retain Job ownership until kill/shutdown/drop, preserving ongoing background work and final-output polling.
4. Job failure must never silently fall back to a successful uncontained Windows start. Fast-exit policy: a very short-lived child (for example cmd /c echo) may exit before assignment. If assignment or verification fails, start still returns an error and publishes no handle; parent exit alone is not proof of safe containment because it may have spawned escaped descendants. If assignment and verification succeed even after exit, the normal pollable terminal handle may be returned. This possible new start failure is an accepted compatibility limitation of the best-effort path and needs a deterministic error-path test plus a natural fast-command regression. Nested host Jobs that prevent assignment produce an actionable start error. Job termination errors must be surfaced by kill; shutdown keeps its existing count API and must at least log failure and drop ownership.

## Ownership And Concurrency

Job handles are non-inheritable and never exposed to tool callers. Child::as_raw_handle is borrowed only while Child is alive. RAII wrapper exclusively owns its Job handle; Send/Sync requires a documented Win32 kernel-handle safety argument. Preserve existing state -> child lock ordering. Job operations must not invert process-map locks or prematurely close a handle in concurrent poll/kill/shutdown.

## Scope And Non-Goals

No filesystem/network restriction, AppContainer, approval bypass, SandboxBackend registration, evaluator migration, Unix process groups, user-visible configuration or new serialized fields. Job Objects do not contain processes delegated through external services and do not retroactively capture pre-assignment descendants.

## Failure And Rollback

All create/configure/assign/verify failures carry operation and OS error. Before map publication, failed start cleans direct child and owned Job with bounded reap. Pipe acquisition failure also cleans the child. RAII closes Job on every path. Rollback removes this Windows-only wrapper/integration and direct dependency, preserving approval contracts and stored data.

## Verification

Windows tests use controlled descendant helpers: the controller waits for start to return (assignment verified), then sends a stdin spawn instruction, waits for descendant readiness, and opens a native SYNCHRONIZE handle while it is still alive. Assert termination using WaitForSingleObject on that held handle, not a PID lookup or sleep alone. Check membership/limit flags, tree kill/shutdown after parent exit, natural-exit descendant survival before cleanup, last-owner drop/kill-on-close, cross-session isolation and failed-start non-publication. Inject create/configure/assign/verify/terminate failures through a per-manager cfg(test) seam around real owned handles; never use global mutable fail switches. Final-owner drop means actual final ownership, including timer closures and outstanding process-record Arcs. Preserve existing poll/stdin/output/buffer tests. Use bounded deadlines and cleanup guards. Run core unit/integration tests and shared check; non-Windows build/CI must compile the unchanged cfg path. OpenSpec strict validation and version/ADR checks apply. Record which platform checks were actually executed, not inferred.

## Review

Three independent reviews (architect, security/correctness, consultant/test) conditionally approve implementation. Accepted requirements: transactional start, exited-parent Job termination, exact OS errors, held native-handle handshake tests, per-instance failure injection, scheduler-unbounded race disclosure and fast-exit fail-closed policy. All must be evidenced before code acceptance. Full dispositions: management/task-system/02_REVIEWS/2026-09-30-pa077-design-review.md.
