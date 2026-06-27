# Spec: Runtime-History State Consistency (v2 — After Adversarial Review)

> **Status: Implemented** (via PA-060/PA-061/PA-062/PA-063, 2026-06-22)
> 本 spec 与 session-cursor-view-contract canonical spec 对应，
> 已通过 PA-061 host-authoritative hard-cut 与 PA-062 browser preview fallback retirement 完成收口。

## Overview

Ensure frontend state consistency between runtime and historical conversation views.  
All changes are in `src/stores/runtime.ts` and `src/types/runtime.ts` — **no Rust backend modification required**.

---

## P1. `canonicalTerminalPhase` Phase 持久化

### Problem

`completed` session loads as `ready` because `persistHistory()` stores `this.phase` which is already `"ready"` after terminal event transition. Loading later cannot distinguish "was completed" from "truly ready".

### Changes

**`src/types/runtime.ts`** — `PersistedRuntimeState`:

```typescript
type PersistedRuntimeState = {
  // ... existing fields ...
  canonicalTerminalPhase?: "completed" | "failed" | "cancelled";
};
```

### Write Path — `persistHistory()` (runtime.ts ~4015):

```typescript
const payload: PersistedRuntimeState = {
  // ... all existing fields ...
  canonicalTerminalPhase: !this.initialRollbackActive && isTerminalPhase(this.phase)
    ? this.phase
    : undefined,
};
```

Create helper:
```typescript
function isTerminalPhase(phase: string): phase is "completed" | "failed" | "cancelled" {
  return phase === "completed" || phase === "failed" || phase === "cancelled";
}
```

### Write Path — `updatePersistedBackgroundSession()` (runtime.ts ~4126):

```typescript
const payload: PersistedRuntimeState = {
  ...persisted,
  phase: patch.phase,
  canonicalTerminalPhase: isTerminalPhase(patch.phase)
    ? patch.phase
    : persisted.canonicalTerminalPhase,
  // ... other fields unchanged ...
};
```

### Read Path — `resolveRestoredPersistedPhase()` (runtime.ts ~2825):

Change signature from 3 to 4 parameters:

```typescript
function resolveRestoredPersistedPhase(
  persistedPhase: RuntimePhase | null | undefined,
  canonicalTerminalPhase: "completed" | "failed" | "cancelled" | null | undefined,
  messages: ChatMessage[],
  turnTraceHistory: TurnTraceRecord[]
): RuntimePhase {
  const restoredPhase = restorePhaseFromTurnHistory(messages, turnTraceHistory);
  if (restoredPhase !== "ready") {
    return restoredPhase;
  }
  // If trace inference falls back to "ready" but we have authoritative canonical info, use it
  if (canonicalTerminalPhase) {
    return canonicalTerminalPhase;
  }
  // Existing fallback: persisted phase for cancelled/failed
  if (persistedPhase === "cancelled" || persistedPhase === "failed") {
    return persistedPhase;
  }
  return "ready";
}
```

### Update all 3 call sites:

`state()` (~line 3614):
```typescript
phase: resolveRestoredPersistedPhase(
  persisted?.phase ?? null,
  persisted?.canonicalTerminalPhase ?? null,
  persisted?.messages ?? [],
  (persisted?.turnTraceHistory ?? []).map(normalizeTurnTraceRecord)
),
```

`applySessionSnapshot()` (~line 4508):
```typescript
this.phase = resolveRestoredPersistedPhase(
  restoredState?.phase ?? null,
  restoredState?.canonicalTerminalPhase ?? null,
  this.messages,
  this.turnTraceHistory
);
```

### Guard for `initialRollbackActive`:

`rollbackToInitialState()` sets `this.initialRollbackActive = true` → `persistHistory()` sets `canonicalTerminalPhase: undefined` (via `!this.initialRollbackActive` guard).

### No change needed for:

- `createSessionRuntimeSnapshot` / `restoreSessionRuntimeSnapshot` — `canonicalTerminalPhase` is read from PersistedRuntimeState, not from SessionRuntimeSnapshot
- Rust backend — entirely frontend-only field

---

## P2. Message ID Stability & Compatibility Check Relaxation

### Problem

`isPersistedStateCompatible` compares both `role` and `content.trim()` — too strict when used for metadata reuse decision in `canReusePersistedState`. Meanwhile `isPersistedMessageShapeCompatible` (role-only) already governs message ID preservation via `canMergePersistedMessages`.

### Changes

**Rename `isPersistedStateCompatible` → `isPersistedMetadataCompatible`** (~line 3331):

```typescript
function isPersistedMetadataCompatible(
  snapshot: SessionSnapshot,
  persisted: PersistedRuntimeState | null
): boolean {
  if (!persisted?.messages?.length || !snapshot.history?.length) {
    return false;
  }
  const persistedHistory = buildTurnHistory(persisted.messages);
  if (persistedHistory.length !== snapshot.history.length) {
    return false;
  }
  return persistedHistory.every(
    (msg, index) => msg.role === snapshot.history[index]?.role
      && msg.content.trim() === snapshot.history[index]?.content.trim()
  );
}
```

The comparison logic is **unchanged** — only the name clarifies intent. Role-only check (`isPersistedMessageShapeCompatible`) already exists at line 3362 and already governs message ID preservation.

**Update caller** `applySessionSnapshot()` (~line 4450):

```typescript
// Before:
const canReusePersistedState = persistedInitialRollbackActive
  || (!historicalRuntimeView && isPersistedStateCompatible(snapshot, persisted));
// After:
const canReusePersistedState = persistedInitialRollbackActive
  || (!historicalRuntimeView && isPersistedMetadataCompatible(snapshot, persisted));
```

`canMergePersistedMessages` already uses `isPersistedMessageShapeCompatible` — no change at line 4452.

---

## P3. `withHistoryOperation` Lightweight Helper

### Problem

Four history operations share ~35 lines of boilerplate each. The helper that encapsulates the common Tauri invoke + post-processing lifecycle should preserve operation-specific error messages.

### Changes

Add store method:

```typescript
async performHistoryOperation<TInvokeArgs extends Record<string, unknown>, TResult extends HistoryControlResult>(
  options: {
    cmd: string;
    invokeArgs: TInvokeArgs;
    normalize: (payload: any, nodes: HistoryNode[], branches: HistoryBranch[]) => TResult;
    selectNodeId?: (result: any) => string | null;
    errorMessagePrefix?: string;
  }
): Promise<TResult | null> {
  try {
    const payload = await safeInvoke(options.cmd, {
      sessionId: this.sessionId,
      expectedCursorVersion: this.cursorVersion,
      ...options.invokeArgs
    });
    await this.loadSessionState(this.sessionId, {
      refreshCatalog: false,
      nodeId: options.selectNodeId?.(payload) ?? null
    });
    const result = options.normalize(payload, this.historyNodes, this.historyBranches);
    this.initialRollbackActive = false;
    this.applyHistoryState(this.sessionId, result);
    this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
      result.historyStateAuditSummary ?? null
    );
    return result;
  } catch (error) {
    this.sessionError = `${options.errorMessagePrefix ?? "操作"}失败：${String(error)}`;
    return null;
  }
}
```

This is an **internal helper** called by the 4 public methods. Each public method still:
1. Validates `sessionId` / parameters
2. Does its own browser-preview guard (`!isTauriAvailable()`)
3. Calls `performHistoryOperation` with operation-specific parameters
4. Browser-preview path (only checkoutHistoryNode has one) remains in the public method

Example refactored `restoreBranchHead`:

```typescript
async restoreBranchHead(branchId?: string | null): Promise<HistoryRestoreResult | null> {
  const sessionId = this.sessionId;
  const targetBranchId = branchId?.trim() || this.activeBranchId;
  if (!sessionId || !targetBranchId) {
    return null;
  }
  if (!isTauriAvailable()) {
    this.sessionError = "浏览器预览模式不支持此操作";
    return null;
  }
  return this.performHistoryOperation({
    cmd: "restore_branch_head",
    invokeArgs: { branchId: targetBranchId },
    normalize: (payload) => normalizeHistoryRestoreResult(payload, this.historyNodes, this.historyBranches),
    errorMessagePrefix: "恢复 branch head"
  });
}
```

Similarly for `forkHistoryNode`, `switchHistoryBranch`.  
`checkoutHistoryNode` keeps its browser-preview fallback (lines 4642-4695) in the public method, calls `performHistoryOperation` only in Tauri path.

---

## P4. `restoreBranchHead` Force "live" + Clear Stale State

### Problem

After `restoreBranchHead`, cursor mode should always be "live" (confirmed: Rust `session.rs:1356` already sets `HistoryCursorMode::Live`). Frontend should double-enforce. Stale running state (`activeTurnId`, `isSubmitting`, etc.) should be cleared.

### Changes

Add to `restoreBranchHead` after successful `performHistoryOperation`:

```typescript
async restoreBranchHead(branchId?: string | null): Promise<HistoryRestoreResult | null> {
  // ... guard checks ...
  const result = await this.performHistoryOperation({ ... });
  if (result) {
    this.historyCursorMode = "live";           // double-enforce: backend already returns live
    this.activeTurnId = null;                   // clear stale running state
    this.activeRunId = null;
    this.isSubmitting = false;
    this.latestExecutionCheckpoint = null;
  }
  return result;
}
```

No changes to `forkHistoryNode`, `switchHistoryBranch`, or `checkoutHistoryNode` — they set mode correctly via backend cursor response.

---

## P5. `submitTurn` Auto-Restore Branch Head Before Message Push

### Problem

Current code flips `historyCursorMode` to "live" locally but doesn't call backend `restore_branch_head`. The `nodeId` in `TurnInput` still points to the historical node.

### Changes

Restructure `submitTurn` (~line 6640):

```typescript
async submitTurn(options?: { images?: TurnInputImage[] }) {
  // Guard against double-submit
  if (this.isSubmitting) {
    return false;
  }

  await this.initializeTurnEvents();
  const providerStore = useProviderStore();
  const settingsStore = useSettingsStore();
  const images = (options?.images ?? []).map((image) => ({ ...image }));
  const message = this.draftMessage.trim();

  // ===== STEP 1: If in historical mode, restore to branch head FIRST =====
  // Only restore if pure "historical" — "historical_dirty" mode means user forked
  // from a checkpoint; auto-restore would discard their fork context.
  const mode = this.historyCursorMode;
  if ((this.initialRollbackActive || (mode === "historical"))
      && isTauriAvailable()) {
    const restored = await this.restoreBranchHead();
    if (!restored) {
      this.sessionError = "无法提交：对话处于历史浏览模式，恢复最新状态失败";
      return false;
    }
  }

  // ===== STEP 2: Reset flags after successful restore =====
  if (this.initialRollbackActive || mode !== "live") {
    this.initialRollbackActive = false;
    this.historyCursorMode = "live";
  }

  // ===== STEP 3: Build payload =====
  const lastAssistantMessage = [...this.messages]
    .reverse()
    .find((entry) => entry.role === "assistant") ?? null;
  const retryingAfterTimeout = isTimeoutErrorDetail(lastAssistantMessage?.errorDetail);
  const providerMessage = buildProviderUserMessage(message, images);
  const displayMessage = buildDisplayedUserMessage(message, images);
  const payload: TurnInput = {
    message: providerMessage,
    displayMessage,
    providerId: providerStore.currentProvider?.id ?? null,
    modelId: providerStore.currentModel?.id ?? null,
    reasoningEffort: providerStore.currentReasoningEffort ?? null,
    workspaceMode: settingsStore.workspaceMode,
    sessionId: this.sessionId,
    nodeId: this.visibleNodeId,              // ← now points to branch head
    history: buildTurnHistory(this.messages),
    images
  };

  // ===== STEP 4: Push user message (safe now — cursor is live) =====
  // ... unchanged from existing code (lines 6665-6692) ...
  // ... existing submit logic unchanged ...
```

**Key changes from v1 spec (adversarial review feedback):**
1. **`isSubmitting` guard** — added at top to prevent double-submit race
2. **`historical_dirty` excluded** — only auto-restore when `mode === "historical"`; `historical_dirty` skips restore (user intentionally forked)
3. **`draftMessage` preserved on failure** — if restore fails, `submitTurn` returns `false` before `this.draftMessage = ""` (line 6705) executes
4. **Message push safe after restore** — `this.messages` is correctly the branch head's messages after restore completes

**Browser Preview**: `isTauriAvailable()` is false, skips restore, falls to `runBrowserPreviewTurn()`.

---

## P6. Background Session Sets Persist + Buffer Truncation

### Problem

`completedSessionSet` and `failedSessionSet` are never persisted — badges lost on reload.  
`persistRunningSessionMap` strips buffers to `""` — mid-stream text lost.  
`MAX_BG_TEXT_BUFFER_CHARS` / `MAX_BG_REASONING_BUFFER_CHARS` exist (and are used at lines 5717-5722) but persist truncation is separate.

### Changes

**`src/stores/runtime.ts`** — `PersistedRuntimeCache` at line 265:

```typescript
type PersistedRuntimeCache = {
  sessions: Record<string, PersistedRuntimeState>;
  runningSessionMap?: Record<string, { turnId: string; phase: RuntimePhase; textBuffer?: string; reasoningBuffer?: string }>;
  completedSet?: Record<string, boolean>;
  failedSet?: Record<string, boolean>;
};
```

**Fix `persistRunningSessionMap`** (~line 3139) — was a module-level function using `this` (BROKEN):

```typescript
// Change signature to accept sets as parameters:
function persistRunningSessionMap(
  map: Record<string, RunningTurn>,
  completedSessionSet: Record<string, boolean>,
  failedSessionSet: Record<string, boolean>
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    const cache = loadPersistedRuntimeCache();
    const stripped: Record<string, { turnId: string; phase: RuntimePhase; textBuffer: string; reasoningBuffer: string }> = {};
    for (const [sid, turn] of Object.entries(map)) {
      stripped[sid] = {
        turnId: turn.turnId,
        phase: turn.phase,
        // Keep last 5K/2K chars instead of emptying
        textBuffer: turn.textBuffer.slice(-5000),
        reasoningBuffer: turn.reasoningBuffer.slice(-2000)
      };
    }
    cache.runningSessionMap = stripped;
    cache.completedSet = { ...completedSessionSet };
    cache.failedSet = { ...failedSessionSet };
    window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
  } catch {
    debugLog("persist:running-map:error");
  }
}
```

**Update caller** `persistHistory()` (~line 4046):

```typescript
// Before:
persistRunningSessionMap(this.runningSessionMap);
// After:
persistRunningSessionMap(this.runningSessionMap, this.completedSessionSet, this.failedSessionSet);
```

**Initialize from cache** — in `initializeSessions()` (~line 5105):

```typescript
const persistedCache = loadPersistedRuntimeCache();
if (persistedCache.completedSet) {
  this.completedSessionSet = { ...persistedCache.completedSet };
}
if (persistedCache.failedSet) {
  this.failedSessionSet = { ...persistedCache.failedSet };
}
```

**`switchSession()`** (~line 4830) — preserve buffer content instead of empty:

```typescript
this.runningSessionMap[this.sessionId] = {
  turnId: this.activeTurnId,
  phase: this.phase,
  textBuffer: this.streamBufferText,       // was ""
  reasoningBuffer: this.streamBufferReasoning
};
```

**DO NOT delete** `MAX_BG_TEXT_BUFFER_CHARS` (line 93) or `MAX_BG_REASONING_BUFFER_CHARS` (line 94) — these are **still actively used** at lines 5717-5722 for in-memory streaming buffer limits. The 5000/2000 truncation in `persistRunningSessionMap` is a persist-time limit only.

---

## P7a. Cleanup: `streamDebug*` Fields

### Problem

8 debug-only fields in `RuntimeState` (lines 169-176):
- No Vue component subscribes to them
- `updateStreamDebugBucket()` already writes to `window.__ponyStreamMetrics`
- They inflate state snapshots

### Changes

**Remove from `RuntimeState`** — lines 169-176:
```
streamDebugDeltaCount: number;
streamDebugFlushCount: number;
streamDebugLastDeltaAtMs: number | null;
streamDebugLastFlushAtMs: number | null;
streamDebugReasoningCharsReceived: number;
streamDebugReasoningCharsFlushed: number;
streamDebugTextCharsReceived: number;
streamDebugTextCharsFlushed: number;
```

**Remove from `restoreSessionRuntimeSnapshot()`** — lines 1967-1974.

**Remove from `resetSessionRuntimeState()`** — lines 3807-3814.

**Remove from `resetStreamDebugMetrics()`** — becomes a no-op (keep method body empty or delete method and callers).

**`flushBufferedStreamText()`** — lines 3882-3900: remove `streamDebug*` increment/calls. `updateStreamDebugBucket()` calls can still pass static values or be moved into a debug-only composable.

**Remove from all `turn:*` event handlers** — any `streamDebug*` references in terminal event handlers.

**Do NOT remove** `window.__ponyStreamMetrics` or `updateStreamDebugBucket()` — they are used by `HomeWorkspace.vue`'s local `streamDebugState` and by the standalone debug UI.

---

## P7c. `eventCursorByTurnId` Unified Update

### Problem

`eventCursorByTurnId` is rebuilt correctly in `applySessionSnapshot()` (line 4492) but NOT rebuilt after all `turnTraceHistory` mutations, causing staleness.

### Changes

Keep `eventCursorByTurnId` as a `RuntimeState` field (line 147) — do NOT convert to a getter (performance concern: `shouldProcessTurnEvent` called on every stream event, getter creates new object each time, causing Vue reactivity re-comparison).

**Add rebuild call** after every `turnTraceHistory` mutation:

1. `applySessionSnapshot()` (~line 4492) — already calls `buildEventCursorByTurnTraceHistory(this.turnTraceHistory)` ✓
2. `upsertTurnTrace()` — after push (~line 5195 area):
   ```typescript
   this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
   ```
3. `commitTurnTraceTimeline()` — after upsert call:
   ```typescript
   // This calls upsertTurnTrace internally; rebuild inside upsertTurnTrace covers this
   ```
4. `checkoutHistoryNode()` Tauri path — after `this.turnTraceHistory = this.turnTraceHistory.filter(...)` (~line 4638):
   ```typescript
   this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
   ```
5. `rollbackToInitialState()` — already sets `this.eventCursorByTurnId = {}` at line 4597 ✓
6. All history ops (restore/fork/switch) — after `loadSessionState` which calls `applySessionSnapshot` which already rebuilds ✓

---

## P7d. `turnTraceHistory` Persistence Stop Writing

### Problem

`turnTraceHistory` is no longer persisted to localStorage (comment: "trace data lives on the backend"). The optional field exists for backward-compatible reads. However: removing it entirely breaks browser preview, and bumping `CACHED_STATE_VERSION` nukes all cache.

### Changes

**Keep the optional field** `turnTraceHistory?: TurnTraceRecord[]` in `PersistedRuntimeState` (lines 189-192) — for backward-compatible reads of old caches.

**Stop writing it** — in `persistHistory()` (~line 4020):
```typescript
// turnTraceHistory intentionally excluded — trace data lives on the backend
```

The comment already exists; no code change needed here.

**Do NOT bump `CACHED_STATE_VERSION`** (keep at `2`, line 262) — bumping would invalidate all cached state including messages, provider info, etc. The presence of `turnTraceHistory` in old caches does not harm correctness because the code already prefers `snapshotTurnTraceHistory` over `restoredState?.turnTraceHistory` (line 4459-4464).

**If browser preview mode needs traces**: `buildRuntimeViewFromPersistedState` (~line 3051) currently uses `persisted?.turnTraceHistory` to populate `snapshot.turnTraceHistory`. This path needs to remain working. Since the field is kept optional and old caches still have it, this works for existing data. New writes won't include it, so browser preview users will see empty traces on page reload — acceptable because `CACHED_STATE_VERSION` bump was removed.

**Update `buildSessionOverviewFromPersistedState()`** (~line 3272):

```typescript
// Before:
const legacyTraceHistory = state.turnTraceHistory ?? [];
// After:
const legacyTraceHistory: TurnTraceRecord[] = state.turnTraceHistory ?? [];
```
(TypeScript type assertion; no behavioral change since `state.turnTraceHistory` is still optional.)

**Update `applySessionSnapshot()`** (~line 4459-4464) — keep existing logic, just remove fallback to `restoredState?.turnTraceHistory`:

```typescript
// Before:
const effectiveTurnTraceHistory = (
  snapshotTurnTraceHistory.length
    ? snapshotTurnTraceHistory
    : historicalRuntimeView
      ? []
      : restoredState?.turnTraceHistory ?? []
).map((trace) => normalizeTurnTraceRecord(trace));

// After:
const effectiveTurnTraceHistory = snapshotTurnTraceHistory.length
  ? snapshotTurnTraceHistory
  : historicalRuntimeView
    ? []
    : []
;
```

---

## Verification & Testing

### Per-Task Verification

| Task | Verification |
|------|-------------|
| P1 | Load `completed` session via Tauri → phase shows `completed`, not `ready` |
| P1 | Load `completed` session in browser preview (with old cache) → phase from `canonicalTerminalPhase` |
| P1 | Old cache without `canonicalTerminalPhase` → falls back to `restorePhaseFromTurnHistory` |
| P1 | `initialRollbackActive` session → phase is `idle`, no canonical persisted |
| P2 | Messages keep original IDs across session reload (when content unchanged) |
| P2 | Name change: `isPersistedStateCompatible` → `isPersistedMetadataCompatible` compiles |
| P3 | All 4 history operations produce same results as before refactoring |
| P3 | Error messages show operation-specific prefix (not generic "操作失败") |
| P4 | `restoreBranchHead` → `this.historyCursorMode === "live"`, `activeTurnId === null` |
| P5 | Submit from `"historical"` mode → auto restores → message pushed after |
| P5 | Submit from `"historical_dirty"` mode (forked) → DOES NOT restore → existing behavior |
| P5 | Submit from `initialRollbackActive` → same behavior as historical |
| P5 | Double-click submit → second call returns `false` via `isSubmitting` guard |
| P6 | Refresh page → background session badges restored |
| P6 | Buffer preserved (last 5000 chars text, 2000 chars reasoning) on refresh |
| P6 | `MAX_BG_TEXT_BUFFER_CHARS` still works (NOT deleted) |
| P7a | Store no longer has `streamDebug*` fields → no console errors |
| P7a | `window.__ponyStreamMetrics` still populated → debug UI works |
| P7c | `eventCursorByTurnId` correct after checkout, restore, fork, switch |
| P7c | `eventCursorByTurnId` rebuilt after `upsertTurnTrace` / `commitTurnTraceTimeline` |
| P7d | New sessions: no `turnTraceHistory` in localStorage |
| P7d | Old cache: backward-compatible read works |
| P7d | Browser preview: traces still load from old cache |

### TypeScript Check

```bash
npx tsc --noEmit
```

### Manual Integration

1. Open a completed conversation → verify phase is `completed` not `ready`
2. Switch to historical node → submit message → verify auto-restore → message appears on live branch
3. Switch away mid-turn → page reload → switch back → verify buffer content preserved
4. All 4 history operations (checkout/restore/fork/switch) → verify consistent state

---

## Change Summary

| Task | Files | Lines Changed | Risk |
|------|-------|--------------|------|
| P1 | `runtime.ts`, `types/runtime.ts` | ~30 | Low |
| P2 | `runtime.ts` | ~10 (rename + caller) | Low |
| P3 | `runtime.ts` | ~60 (new helper + 4 refactors) | Medium |
| P4 | `runtime.ts` | ~10 (restoreBranchHead only) | Low |
| P5 | `runtime.ts` | ~30 (submitTurn restructure) | Medium |
| P6 | `runtime.ts`, `types/runtime.ts` | ~40 | Low |
| P7a | `runtime.ts` | ~30 (remove fields + refs) | Low |
| P7c | `runtime.ts` | ~20 (add rebuild calls) | Low |
| P7d | `runtime.ts` | ~10 (applySessionSnapshot) | Low |
| **Total** | | **~240** | |

### Implementation Order

```
P2 → P1 → P4 → P5 → P3 → P6 → P7a → P7c → P7d
```
P2 first (rename before using), P1 (phase), P4+P5 (restore+submit), P3 (helper), P6+P7 (cleanup).
