# PA-103 Signed desktop self-update — tasks

## Local contract gate (this change; fully verifiable without production key/endpoint)

- [x] Evidence boundary: SSH reference is source/config observation only; no claim of a proven remote release.
- [x] Plan/spec: proposal/design/tasks + ADR 0019 record the two-gate model, source separation, trust anchors, platform matrix and non-rollback semantics; three-way plan review PASS after revision.
- [x] JS deps `@tauri-apps/plugin-updater` + `@tauri-apps/plugin-process` added via npm; package-lock updated.
- [x] Rust deps `tauri-plugin-updater`/`tauri-plugin-process` added (workspace root Cargo.lock contains both; src-tauri is a workspace member); `npm run cargo:check` compiles clean.
- [x] Plugins registered unconditionally in `src-tauri/src/lib.rs` so capability permission IDs resolve; tauri.conf.json updater block intentionally ABSENT (no placeholder key/endpoint); frontend adapter disabled via `SIGNED_UPDATER_ENABLED=false`.
- [x] Typed updater adapter `src/lib/tauri-updater.ts`: dynamic plugin calls only; exported pure `mapDownloadEvent`/`parseSignedCandidate`; events matched case-insensitively (`Started/Progress/Finished`); unknown content length safe; no URL/endpoint parameter.
- [x] Store `src/stores/update.ts`: `githubLatest` (check-only) separated from `signedCandidate`; typed-handle-only install; check/install in-flight guards; pending-restart vs relaunch-failed semantics; failed install clears candidate, safe fixed error copy, raw error only in console.
- [x] UI `ConfigGeneralSection.vue`: install CTA only when signed candidate available; browser/dev no-install; progress/error/pending-restart/relaunch-failed testids; GitHub check button disabled while installing; duplicate button removed.
- [x] Capabilities: exact `updater:allow-check`, `updater:allow-download-and-install`, `process:allow-restart`; no shell/process execute; gen/schemas regenerated and in sync.
- [x] Tests: `tests/tauri-updater.spec.ts` (fail-closed + event/candidate parsing), `tests/update-signed-store.spec.ts` (state machine/concurrency/clamp/failure), `tests/ConfigGeneralSectionSignedUpdate.spec.ts` (UI four states + browser no-install), existing update tests unchanged.
- [x] Gates run and recorded: `npm run version:check` PASS (tauri 0.1.94 four places, core 0.1.92); targeted vitest PASS; full vitest 569 pass / 10 skip / 1 pre-existing flake (MarkdownRenderer passes isolated, files unmodified); `npm run typecheck` PASS; `npm run build` PASS; `npm run cargo:check` PASS; vitest required danger-full-access due to sandbox EPERM on Node child spawn (documented boundary).
- [x] Three-way implementation review (correctness/security/test) CONDITIONAL PASS; all P1 fixes applied (placeholder config removed, dead param removed, error copy sanitized, event casing fixed, duplicate button removed, tests added); final re-verification PASS.
- [x] Closeout: ADR 0019 indexed, review records written, dashboard/board/task card/session log updated, OpenSpec change archived, commit summary prepared (no commit unless requested).

## Release-owner gate (deferred; explicitly NOT DONE in local contract)

- [ ] Provision a real Pony Agent signing key outside git; publish a repository-fixed HTTPS endpoint; add CI release workflow with protected secrets; pin exact plugin versions.
- [ ] Add `plugins.updater` (real pubkey/endpoints) and `bundle.createUpdaterArtifacts`; regenerate capability schemas; verify exact `plugin:updater|check`/`download_and_install` IPC payloads against the pinned plugin source; document redirect/proxy policy (default safe, `dangerous_*` off).
- [ ] Implement/review `prepare_update_exit` cleanup and active-turn confirmation before install (NOT implemented in the local contract; currently the installer would replace a running app without a confirmation gate).
- [ ] Decide whether startup auto-runs the signed check (local contract is manual-only via the settings card).
- [ ] Sign artifacts, publish manifest atomically after tag/four-version/target matrix validation; hard-fail on missing key, signature failure, public-key mismatch or missing target.
- [ ] Windows x64 (and agreed matrix) signed smoke: tamper/wrong-key rejection, install/relaunch version evidence, user-data preservation, offline/partial-download and crash recovery; logs redacted.
- [ ] Define key rotation/revocation, artifact retention, withdrawal/recovery runbook and first-rollout manual bootstrap for pre-updater installs; then flip `SIGNED_UPDATER_ENABLED` and fill config.

## Acceptance (split)

Local gate: plumbing compiles, tests pass, no fake key/endpoint/artifact, install disabled and fail-closed, GitHub check-only preserved, no shell/arbitrary URL/window.open install path, no production-feature claim.

Release gate: real endpoint/public key/artifacts/one-click install apply only after release-owner evidence; until then production one-click install is NOT DONE.
