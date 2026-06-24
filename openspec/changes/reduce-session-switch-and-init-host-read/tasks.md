# Tasks

## Completed
- [x] Diagnose switch-path, host-read, and workspace-frame bottlenecks
- [x] Remove automatic host refresh from cached session switches
- [x] Remove synchronous catalog refresh from `createSession()`
- [x] Add local-first staged hydration for workspace rendering
- [x] Eliminate duplicate session entries and duplicate sidebar keys
- [x] Spec triaged by 3-dimension review (architecture, UX, testability)

## Remaining: initializeSessions() cache-first with checkpoint recovery

- [ ] Implement `cachedStateVersion` schema field and validation in `loadPersistedRuntimeState()`
  - Acceptance: corrupted-cache path falls through to placeholder without exception
- [ ] Implement checkpoint insufficiency predicate: `state.checkpoint == null || state.phase === "idle"` triggers host read
  - Acceptance: `initializeSessions()` with valid persisted checkpoint does NOT call `loadSessionRuntimeViewState`
  - Acceptance: `initializeSessions()` with insufficient persisted state calls lighter host read (not full runtime view)
- [ ] Replace implicit `null` branching with explicit `SessionInitializationStrategy` discriminated union
  - Acceptance: all three branches (local-cache, host-checkpoint, cold-host-read) handled explicitly with no `null`-as-flag

## Remaining: Error and edge-case resilience

- [ ] Add cache validation + fallback in `switchSession()` for corrupted persisted state
  - Acceptance: JSON parse error or version mismatch triggers uncached placeholder path
- [ ] Add `localStorage` unavailable handler (try-catch + degrades to uncached)
  - Acceptance: initialization and switch complete without error when `localStorage` throws
- [ ] Add host read timeout (15s) for hydration with inline error + retry
  - Acceptance: uncached switch with mocked slow host read shows timeout state after 15s
- [ ] Add stale session reconciliation in sidebar
  - Acceptance: session deleted on backend shows as dimmed with dismiss affordance
- [ ] Add running turn state persistence across app restart
  - Acceptance: `runningSessionMap` restored from localStorage in `initializeSessions()`

## Remaining: Regression and validation

- [ ] Add concurrent rapid-switch test: A → B → C, verify final state is C
  - Acceptance: `sessionOperation` not stuck, final session is C, A logged in debug output
- [ ] Add corrupt-cache test: invalid JSON in persisted state → fallback to placeholder
- [ ] Add localStorage-unavailable test: mock `localStorage.setItem` to throw → verify graceful degradation
- [ ] Add hydration-timeout test: mock host read delay >15s → verify error state + retry
- [ ] Add stale-session test: backend returns list without cached session → verify dimmed entry
- [ ] Re-run full regression suite for runtime store and workspace behavior
- [ ] Validate real multi-session long-task interaction (manual: switch 3+ sessions while turn active)

## Validation
- `npx vue-tsc --noEmit`
- `./node_modules/.bin/vitest.cmd run tests/runtime-store.spec.ts`
- `./node_modules/.bin/vitest.cmd run tests/HomeWorkspace.spec.ts`
- `./node_modules/.bin/vitest.cmd run tests/HomeSessionSidebar.spec.ts` (if exists)
- 后端编译由 `cargo:check` 单独验证，与前端行为无关
