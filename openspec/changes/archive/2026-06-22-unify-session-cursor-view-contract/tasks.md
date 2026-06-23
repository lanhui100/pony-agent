# Tasks

## Phase 1: Contract Definition (PA-060)

- [x] Define `session-cursor-view-contract` canonical spec
- [x] Separate `HistoryGraph / Cursor / View` three-layer model
- [x] Define authority mode (`host_authoritative` / `local_preview`)
- [x] Define host-projected read-model fields
- [x] Define cursor versioning and stale mutation conflict
- [x] Run 3-way spec review (architecture / multi-surface / migration)

## Phase 2: Host Authoritative Hard Cut (PA-061)

- [x] Remove frontend local cursor fallback from host-backed `loadSessionState()`
- [x] Remove `previousHistoryState` compensation in host path
- [x] Host authoriative runtime view now clears stale local historical state
- [x] Run targeted frontend tests

## Phase 3: Browser Preview Degradation (PA-062)

- [x] Disable `restoreBranchHead / forkHistoryNode / switchHistoryBranch` in preview mode
- [x] Return `authority_mode: "local_preview"` in preview runtime views
- [x] Run preview mode tests

## Phase 4: Cursor Versioning (PA-063)

- [x] Add `cursor_version` to `HistoryCursor`
- [x] Add `expected_cursor_version` to all four history-control commands
- [x] Reject stale mutations with explicit conflict error
- [x] Frontend store passes `cursorVersion` in Tauri commands
- [x] Frontend surfaces conflict via `sessionError`
- [x] Run cursor versioning tests

## Phase 5: Testing & Validation

- [x] Frontend targeted tests pass
- [x] Rust session_regression / tool_router_regression / provider_registry_regression pass
- [x] npm run verify passes
- [x] Full CI verification
