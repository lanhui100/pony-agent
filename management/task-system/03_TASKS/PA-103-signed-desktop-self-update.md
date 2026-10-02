# PA-103 Signed desktop self-update

- **Task ID**: PA-103
- **Status**: In Progress — Discovery complete; plan/spec review pending
- **Complexity**: C (cross-module security-sensitive release behavior)
- **Priority**: P1
- **Owner**: Lead + implementation-engineer
- **Created**: 2026-10-02
- **Spec**: `openspec/changes/2026-10-02-signed-desktop-updater/`
- **Reviews**: `management/task-system/02_REVIEWS/` (to be added after real reviews)

## Background

The current desktop checks GitHub Releases and opens a release page, but cannot perform a verified one-click install. SSH discovery of `dev:~/pproxy/desktop` found a Tauri updater implementation using `@tauri-apps/plugin-updater`, `downloadAndInstall`, progress state, `@tauri-apps/plugin-process` relaunch, updater/process permissions, signed updater artifacts and an idempotent pre-exit cleanup command.

## Goal

Implement a signed Tauri desktop update flow for Pony Agent while preserving browser check-only compatibility and existing fail-closed URL/sandbox boundaries.

## Scope

- Tauri updater/process dependencies, registration, capabilities and signed updater configuration.
- Frontend adapter/store/UI state for check, progress, install and relaunch.
- Tests, release/config documentation, OpenSpec and ADR.

## Non-goals

Unsigned downloads, shell-based replacement, automatic silent install, rollback engine, private key storage, widening URL allowlists, or changes to agent-core/sandbox policy.

## Acceptance criteria

1. Tauri builds have updater plugin wiring, HTTPS endpoint(s), public key, updater artifact generation and minimal updater/process permissions.
2. A user can click one update action in the settings card; the UI shows checking/progress/error states and invokes signed `downloadAndInstall` then relaunches.
3. Browser/dev mode remains check-only and never attempts installation.
4. Concurrent checks/install actions are idempotently guarded; failed install preserves truthful state and does not relaunch.
5. No private signing key, arbitrary download URL, shell command or `window.open` install fallback enters the repository.
6. Existing version synchronization and security tests remain green; targeted updater tests cover success/failure/concurrency/browser behavior.
7. Three independent reviewers approve the plan/spec and implementation, with accepted findings recorded.

## Risks / rollback

The signed endpoint/public key and release CI must agree. If release infrastructure cannot be safely configured in this checkout, keep the endpoint contract documented and disable installation rather than shipping an unsigned fallback. Rollback is file-level: remove updater plugin/config/UI integration and retain the existing check-only path.

## Current progress

- Workspace baseline inspected; substantial unrelated uncommitted changes exist and must not be reset.
- SSH access to `dev` succeeded; remote implementation evidence collected.
- OpenSpec proposal/design/tasks created.
- Next action: complete three-way plan/spec adversarial review, then revise before implementation.

## Resume hint

Open `openspec/changes/2026-10-02-signed-desktop-updater/design.md`, then read the three PA-103 plan-review files once created. Do not start code changes until the review gate is marked PASS.
