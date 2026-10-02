# PA-103 Signed desktop self-update

- **Task ID**: PA-103
- **Status**: Done — local contract gate closed (2026-10-02); release-owner gate open (see below)
- **Complexity**: C (cross-module security-sensitive release behavior)
- **Priority**: P1
- **Owner**: Lead + implementation-engineer
- **Created / Closed**: 2026-10-02
- **Spec**: `openspec/changes/archive/2026-10-02-signed-desktop-updater/`
- **Reviews**: `management/task-system/02_REVIEWS/2026-10-02-pa103-plan-{architecture,security,release}.md`, `2026-10-02-pa103-implementation-{correctness,security,test}.md`

## Background

The current desktop checks GitHub Releases and opens a release page, but cannot perform a verified one-click install. SSH discovery of `dev:~/pproxy/desktop` observed a Tauri updater pattern (plugin calls, configuration, permissions) as source/config reference only — it did not prove a successful remote release, signature, install, cleanup or relaunch. Pony Agent adopts the pattern behind an explicit local-contract gate and a separate release-owner enablement gate.

## Goal

Implement signed Tauri updater plumbing for the desktop while preserving browser check-only compatibility and existing fail-closed URL/sandbox boundaries. With no real Pony Agent endpoint/key/CI available, installation stays disabled and fail-closed.

## Scope (local gate, done)

- JS deps `@tauri-apps/plugin-updater` + `@tauri-apps/plugin-process`; Rust deps `tauri-plugin-updater`/`tauri-plugin-process` (workspace lock updated).
- Plugins registered in `src-tauri/src/lib.rs`; `tauri.conf.json` updater block intentionally ABSENT (no placeholder trust anchor); capabilities grant exactly `updater:allow-check`, `updater:allow-download-and-install`, `process:allow-restart`; gen/schemas regenerated.
- `src/lib/tauri-updater.ts`: typed opaque-handle adapter, `SIGNED_UPDATER_ENABLED=false`, case-insensitive download event mapping, defensive candidate parsing, no URL/endpoint parameter, no runtime override.
- `src/stores/update.ts`: `githubLatest` (check-only) separated from `signedCandidate`; check/install in-flight guards; pending-restart vs relaunch-failed; failed install clears uncommitted candidate; safe error copy.
- `src/components/config/ConfigGeneralSection.vue`: install CTA only for signed candidate; progress/error/pending-restart/relaunch-failed states; browser/dev no-install; duplicate button removed.
- Tests: adapter (7), store signed flow (14), component signed UI (5), existing update tests unchanged (49). Full vitest 592 passed / 10 skipped / 41 files (final run green; one pre-existing MarkdownRenderer timing flake passes isolated and is unrelated).

## Non-goals (unchanged)

Unsigned downloads, shell-based replacement, automatic silent install, rollback engine, private key storage, widening URL allowlists, or changes to agent-core/sandbox policy.

## Acceptance criteria (local gate — met)

1. Rust/JS compile, version four-place sync 0.1.94 (core 0.1.92).
2. Fail-closed: no fake key/endpoint/artifact; GitHub-only metadata never yields an install CTA; browser/Vite/Tauri-dev no install path.
3. Typed-handle-only install; no shell, arbitrary URL or `window.open` install fallback.
4. Three-way plan and implementation reviews passed (conditional findings all adopted and re-verified).

## Release-owner gate (OPEN — NOT DONE locally)

- Real signing key (out of git), fixed HTTPS endpoint, protected CI secrets, pinned plugin versions.
- `plugins.updater` config + `createUpdaterArtifacts`; exact IPC payload re-verification; redirect/proxy policy.
- `prepare_update_exit` cleanup + active-turn confirmation (NOT implemented locally); startup auto-check decision.
- Signed artifacts, atomic manifest publication, Windows x64 signed smoke (tamper/wrong-key/install/relaunch/user-data), key rotation/revocation, crash recovery, first-rollout bootstrap for pre-updater installs; then flip `SIGNED_UPDATER_ENABLED` and fill config.

## Verification evidence

`npm run version:check` PASS · targeted updater vitest 91/91 PASS · full vitest 592/602 PASS (10 skipped) · `npm run typecheck` PASS · `npm run build` PASS · `npm run cargo:check` PASS. Vitest/vite needed `danger-full-access` because the DSH sandbox blocks Node child-process spawn with piped stdio (EPERM; documented boundary).

## Resume hint

To enable production updates: follow the release-owner gate items in `openspec/changes/archive/2026-10-02-signed-desktop-updater/tasks.md`, then flip `SIGNED_UPDATER_ENABLED` in `src/lib/tauri-updater.ts` and add the real updater block in `src-tauri/tauri.conf.json`. All plan/implementation reviews and the session log are under `management/task-system/02_REVIEWS/` and `99_LOGS/`.
