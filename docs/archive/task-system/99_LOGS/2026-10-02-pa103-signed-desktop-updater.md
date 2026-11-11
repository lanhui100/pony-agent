# 2026-10-02 PA-103 Signed desktop self-update — session log

## What was done

- Discovery (ssh `dev`): inspected `~/pproxy/desktop` updater implementation (`useUpdater.ts`, `tauri.conf.json`, `capabilities/default.json`, `lib.rs`) as **source/config reference only**; no claim of a proven remote release/signature/install/relaunch.
- C1 plan/spec: created `openspec/changes/2026-10-02-signed-desktop-updater/{proposal,design,tasks}.md`, ADR 0019 (`docs/decisions/0019-signed-desktop-updater-local-contract.md`, Status implemented), task card PA-103. Three-way plan review (architecture/security/test) initially FAIL/HOLD → revised to two-gate model (local contract vs release-owner gate), source separation, trust anchors, platform matrix → PASS.
- C2 implementation (local gate): JS/Rust updater+process deps, plugin registration in `src-tauri/src/lib.rs`, exact capabilities, `src/lib/tauri-updater.ts` (typed adapter, `SIGNED_UPDATER_ENABLED=false`, case-insensitive event mapping, defensive parsing), `src/stores/update.ts` (separated signed state machine, guards, safe errors), `src/components/config/ConfigGeneralSection.vue` (install CTA/progress/pending-restart/error, no duplicate), tests (`tauri-updater.spec.ts`, `update-signed-store.spec.ts`, `ConfigGeneralSectionSignedUpdate.spec.ts`).
- Three-way implementation review (correctness/security/test): CONDITIONAL PASS; all P1 adopted: removed empty `plugins.updater` placeholder from `tauri.conf.json`, removed dead `allowDowngrades` param, sanitized error copy, fixed event casing, cleared candidate on failed install, kept install button visible while downloading, added component/adapter/store tests, aligned ADR/design/tasks.
- Closeout: review records (3 plan + 3 implementation), task card Done, board/dashboard updated, change archived to `openspec/changes/archive/2026-10-02-signed-desktop-updater/`.

## Files changed (PA-103 scope)

- `package.json`, `package-lock.json` (deps), `Cargo.lock` (root workspace; plugins), `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json` (updater block removed), `src-tauri/capabilities/default.json`, `src-tauri/gen/schemas/*`
- `src/lib/tauri-updater.ts` (new), `src/stores/update.ts`, `src/types/update.ts`, `src/components/config/ConfigGeneralSection.vue`
- `tests/tauri-updater.spec.ts`, `tests/update-signed-store.spec.ts` (new), `tests/ConfigGeneralSectionSignedUpdate.spec.ts` (new)
- Docs/governance: ADR 0019, task card, 3+3 review records, dashboard/board, session log, OpenSpec change (archived)

## Gate evidence

- `npm run version:check` PASS (tauri 0.1.94 four places; core 0.1.92)
- Targeted updater tests 91/91 PASS; full vitest final run 592 passed / 10 skipped / 41 files (one pre-existing MarkdownRenderer timing flake passes isolated; its files are unmodified by PA-103)
- `npm run typecheck` PASS; `npm run build` PASS; `npm run cargo:check` PASS
- Environment note: vitest/vite required `danger-full-access` because the DSH sandbox blocks Node child-process spawn with piped stdio (EPERM — documented boundary), not a code failure.

## Next steps (release-owner gate, NOT done)

Real signing key (out of git), fixed HTTPS endpoint, protected CI secrets, pinned plugin versions; `plugins.updater` config + `createUpdaterArtifacts`; IPC payload re-verification; `prepare_update_exit` + active-turn confirmation; startup auto-check decision; signed artifacts/atomic manifest/Windows x64 smoked smoke; key rotation/revocation; crash recovery; first-rollout bootstrap; then flip `SIGNED_UPDATER_ENABLED` and fill config.

## Resume hint

Open `management/task-system/03_TASKS/PA-103-signed-desktop-self-update.md` (release-owner gate section) or `openspec/changes/archive/2026-10-02-signed-desktop-updater/tasks.md`.

## Governance notes

Flagged inconsistency (not fixed here, separate task): `AGENTS.md`/Dashboard claim `npm run verify` includes OpenSpec check, but `package.json` `verify` does not and `scripts/check-openspec.ps1` is deprecated/exit 0; actual ADR gate is `scripts/verify-decisions.ps1`. Workspace baseline had substantial unrelated uncommitted changes (PA-077, .meta, AGENTS.md etc.); PA-103 changes are isolated but the tree is not clean — commit preparation should separate PA-103 scope.
