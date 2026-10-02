# PA-077 实现对抗审核（2026-09-30）

## 状态

Final / conditional pass within approved best-effort scope.

- First-pass correctness/security reviewers independently found P1 failed-start cleanup and evidence gaps.
- Remediation added bounded Windows kill/reap diagnostics, Job close fallback on explicit termination errors, native DuplicateHandle + WaitForSingleObject failure observers, deterministic exit-before-assign seam, pipe rollback evidence, exited-parent explicit kill coverage, and Job handle inheritance-flag verification.
- Fresh fixed-snapshot correctness review `36d68fee-ec82-4e5b-a5c6-b8ad82031449`: conditional pass; no remaining P0/P1 lifecycle blocker.
- Security reviewer correctly retains the strict-containment objection as an accepted scope limitation: the scheduler-dependent std spawn→assign window is not a security boundary and is documented in ADR/OpenSpec. PA-077 does not register a SandboxBackend or authorize autonomous Run.

## Findings and dispositions

| Finding | Disposition |
|---|---|
| Assignment/verification rollback could drop an uncontained child | **Accepted and fixed**: `kill_and_reap` is bounded and reports kill/wait/timeout; `failed_start_cleanup` closes the Job fallback and preserves diagnostics. Native duplicated handles prove assign/verify failure child exit. |
| Pipe rollback did not reap/report | **Accepted and fixed** on Windows for stdout/stderr with bounded cleanup and combined diagnostics; cfg(test) observer proves stdout path. |
| No deterministic early-exit evidence | **Accepted and fixed**: per-manager `wait_before_assign` waits for exact child exit before injected assign failure; test asserts no publication and native signaled state. Natural fast command remains covered. |
| Explicit Job termination error could leave descendants | **Accepted and fixed**: exited kill, running terminate reap-error and normal paths call `Job::close()` kill-on-close fallback and return errors; timer and workspace command callers now log/return cleanup failures rather than silently suppressing them. |
| Exited-parent kill branch untested | **Accepted and fixed**: dedicated parent-exits-then-explicit-kill test plus shutdown coverage. |
| Non-inheritable Job handle evidence | **Accepted and fixed**: `GetHandleInformation` asserts `HANDLE_FLAG_INHERIT=0`; implementation clears inheritance under synchronized Job handle ownership. |
| Nested host Job/breakaway and timer/outstanding-Arc stress | **Not added**: follow-up evidence only. Static flags omit breakaway permissions; scope explicitly remains best-effort lifecycle containment, not strict containment. |
| Spawn-to-assignment race | **Not fixed by design**: scheduler-dependent/unbounded window is explicitly disclosed and excluded from security-boundary claims; strict suspended/native startup requires a separate task. |

## Validation evidence

- Historical implementation snapshot only: the recorded Windows/core/process/tools/version/ADR results were produced before Gate0 reconciliation and are not a current exact-diff full-green claim.
- Current Gate0 replacement evidence is tracked in `2026-09-30-pa077-gate0-closeout-review.md`: `npm run version:check` passed; no `npm run openspec:check` script exists; canonical strict validation is 54 passed while active PA-103 fails; cargo fmt has pre-existing repository-wide differences; exact targeted rerun is ACL-blocked; Linux/macOS CI is configured but not locally executed.

## Final verdict

**Conditional pass / acceptance within PA-077 scope.** The implementation provides best-effort Windows process-tree lifecycle containment and preserves sandbox fail-closed authorization. Do not describe it as strict containment or a filesystem/network sandbox. No commit was created.
