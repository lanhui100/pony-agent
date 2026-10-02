# PA-103 implementation review — correctness & maintainability

- Date: 2026-10-02
- Reviewer: independent code reviewer (correctness focus)
- Verdict: **CONDITIONAL PASS → PASS after fixes**

## Findings adopted

- P1 event casing: plugin channel emits `Started/Progress/Finished` (capitalized); adapter matched lowercase. Fixed with case-insensitive normalization via exported pure `mapDownloadEvent`; tests added for both casings.
- P1 duplicate install button: same testid rendered twice in `ConfigGeneralSection.vue`. Duplicate removed; region re-indented; install button now stays visible (disabled, "正在升级…") while downloading.
- P1 test gaps: added adapter unit tests (`mapDownloadEvent`, `parseSignedCandidate` defensive matrix), store tests (check failure, cross-concurrency, late progress, clamp, candidate-cleared-on-failure), and component UI tests (`ConfigGeneralSectionSignedUpdate.spec.ts`: available→install, downloading→progress/disabled, error→safe copy + no relaunch, relaunch-failed, browser no-install).
- P2/P3: safe fixed error copy with raw error only in console; progress guard `contentLength > 0`; GitHub check button disabled while installing; `prepare_update_exit`/active-turn confirmation explicitly marked release-gate NOT-implemented in design/tasks.

## Verified by reviewer

- Fail-closed tri-guard (`SIGNED_UPDATER_ENABLED=false` + `PROD` + Tauri) short-circuits check/install; browser/dev zero install path; existing GitHub check-only semantics preserved (update-check/update-store/ConfigGeneralSectionUpdate tests unchanged and green).

## Residual risk (release gate)

Real signature/install/relaunch correctness requires signed fixtures and a Windows smoke; adapter IPC payload shape is to be re-verified against the pinned plugin version at release gate. Local contract does not claim production one-click install.
