# Design

## Frontend session lifecycle

- `switchSession()` SHALL treat persisted local cache as the primary truth source for immediate UI switching.
- Cached sessions SHALL switch foreground state without automatic `load_session_runtime_view` or `list_sessions` follow-up reads. A lightweight async staleness check SHALL be dispatched after the switch completes.
- Uncached sessions SHALL still switch immediately into a lightweight placeholder state, without blocking on host reads. A background host read SHALL be auto-dispatched after placeholder render.
- Corrupted or schema-mismatched cache SHALL fall through to the uncached placeholder path, with a diag warning but no exception.
- Rapid sequential switches SHALL use `sessionSwitchToken` to discard stale responses; only the final target session's state SHALL be applied.

## Initialization strategy

- `initializeSessions()` SHALL prefer persisted local runtime state when available.
- The insufficiency predicate for falling back to host read: persisted state is insufficient iff `localState.checkpoint == null || localState.phase === "idle"`.
- Host reads during initialization SHALL be minimized and reserved for cases where no local persisted state exists or where active checkpoint recovery facts are unavailable locally.
- Recovery and terminal phase semantics MUST remain intact.
- On `localStorage` unavailability (quota exceeded, private mode, disk full), a diag warning SHALL be emitted and the cold host-read fallback path SHALL be used.

## Cache validation

- Persisted state SHALL include a `cachedStateVersion` field.
- On read, the version SHALL be checked. On mismatch, the cache SHALL be treated as corrupted and the uncached path SHALL be taken.
- Missing required fields (`sessionId`, `history`, `phase`) SHALL also trigger the corrupted-cache path.

## Sidebar stability

- Session sidebar rendering SHALL dedupe entries by `conversationId` before render. Only the first occurrence by array index SHALL be rendered.
- Session creation SHALL generate collision-safe ids and MUST NOT reinsert the previous active session overview twice.
- Sessions that no longer exist on the backend SHALL be rendered as dimmed/deleted entries with an affordance to dismiss.

## Workspace hydration

- Workspace transcript rendering SHALL be staged: first a `SessionPlaceholderSkeleton` (for uncached) or `SessionLoadingSkeleton` (for cached hydration), then the full transcript list.
- During session hydration, the transcript list MAY be withheld while a lightweight loading state is shown. On completion, the loading state SHALL be replaced with the full transcript using Vue `<Transition>`.
- A 15-second timeout SHALL apply to hydration. On timeout, an inline error with retry SHALL be shown.
- The loading state SHALL NOT block sidebar interaction.

## Background turn management

- `runningSessionMap` entries SHALL be persisted to `localStorage` alongside session runtime state.
- On app restart, persisted `runningSessionId` and `turnId` SHALL be restored into `runningSessionMap`.
- On switching to a session with a background turn, a lightweight host status check SHALL verify the turn's actual state (`running`, `completed`, `failed`). The frontend SHALL NOT assume the persisted phase is current.
- Switching back to a session with a background turn SHALL show a non-intrusive banner indicating the turn status.

## Write-back semantics

- `persistHistory()` SHALL be called before every `switchSession()` and `createSession()`.
- `persistHistory()` SHALL wrap `localStorage` operations in try-catch. On failure, a diag warning SHALL be emitted and the switch SHALL proceed without persistence.
- On `beforeunload`, `persistHistory()` SHALL be called to save current session state and running turn markers.

## Diagnostics

- Performance diagnostics SHOULD only emit when thresholds are exceeded.
- Host read and workspace frame timing remain available for targeted regression analysis.
- Cache hit/miss and corruption events SHALL be logged at `debug` level.

## Non-goals reaffirmed

- 本 change 不直接把 `AgentRuntime` 改成 fully async actor model
- 本 change 不完成 `reqwest::blocking` -> async `reqwest` 的 provider 全量改造
- 本 change 不重写 backend session storage contract
- 本 change 不引入跨窗口/跨进程缓存一致性协议（version vector 方案留给后续 change）
- 本 change 不新增后端 Tauri command（轻量宿主读取复用已有 `load_session_runtime_view`，但调用方按需选择字段路径）
