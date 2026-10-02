## 1. Design

- [x] 1.1 Create PA-077 task, proposal, design, delta spec and ADR 0018.
- [x] 1.2 Complete three independent adversarial design reviews and record disposition.

## 2. Windows Containment

- [x] 2.1 Add Windows-only windows-sys dependency/features and Job RAII wrapper.
- [x] 2.2 Integrate fail-closed setup/assignment and failed-start cleanup.
- [x] 2.3 Integrate kill/shutdown/last-owner cleanup including exited-parent descendants.
- [x] 2.4 Update process/sandbox documentation without changing approval availability.

## 3. Verification And Closeout

- [x] 3.1 Add deterministic descendant and error-path tests with bounded cleanup.
- [x] 3.2 Complete independent implementation reviews and resolve findings.
- [x] 3.3 Run core regression suite, formatting, shared check, version/OpenSpec/ADR validation.

> 3.3 records the historical implementation snapshot and its then-available validation evidence. It is not a claim that the current mixed working tree is full-green; current Gate0 blockers and replacements are recorded in `management/task-system/02_REVIEWS/2026-09-30-pa077-gate0-closeout-review.md`.

- [ ] 3.4 Synchronize canonical spec, archive change, task board, dashboard and session log.

> Note: Windows-only behavior was implemented and reviewed statically; the current host ran the
> Windows-targeted cfg(test) code path, but hosted Linux/macOS CI remains the authoritative
> non-Windows cross-platform check and was not executed locally.
