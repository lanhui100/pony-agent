import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { isAskExpired, useAskStore, ASK_POLL_INTERVAL_MS } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

// Wave-15 red-phase acceptance (frozen contract `.dev-team/contracts/wave-15-ask-cas.md`):
// C2-1 ~ C2-3 — a locally expired Ask must never be submitted to the host CAS path; it must be
// reconciled through `ask_expire` + `ask_list_pending` with an explicit 「过期」 message, while a
// still-live Ask keeps submitting with the version observed at listing time.

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

function createPendingAsk(partial: Partial<PendingAsk> = {}): PendingAsk {
  return {
    requestId: partial.requestId ?? "ask-1",
    requestKind: partial.requestKind ?? "interaction",
    sessionId: partial.sessionId ?? "session-1",
    runId: partial.runId ?? "run-1",
    turnId: partial.turnId ?? "turn-1",
    callId: partial.callId ?? "call-1",
    descriptorSnapshotId: partial.descriptorSnapshotId ?? "snapshot-1",
    descriptorId: partial.descriptorId ?? "builtin:ask_user",
    finalArgsDigest: partial.finalArgsDigest ?? "args-digest",
    policyDigest: partial.policyDigest ?? "policy-digest",
    nonce: partial.nonce ?? "nonce-1",
    version: partial.version ?? 1,
    expiresAtMs: partial.expiresAtMs ?? Date.now() + 60_000,
    state: partial.state ?? "pending",
    prompt: partial.prompt ?? "继续？",
    options: partial.options ?? ["是", "否"]
  };
}

function commands(): string[] {
  return tauriMocks.mockSafeInvoke.mock.calls.map(([command]) => String(command));
}

/**
 * The IPC names that make up the refresh/expiry conversation, in call order. Filtering out
 * unrelated commands (graph/answer/cancel) keeps the C2-6 sequence assertion exact without
 * coupling it to incidental traffic.
 */
function reconcileCommands(): string[] {
  return commands().filter((command) =>
    ["ask_list_pending", "ask_expire"].includes(command)
  );
}

/**
 * Host mock for the expired-Ask path: the first `ask_list_pending` still returns the stale
 * (terminal, re-listed) request — exactly the symptom the user hit — and every later list returns
 * the reconciled, empty truth after `ask_expire`.
 */
function mockExpiredAskHost(expired: PendingAsk) {
  let listCalls = 0;
  tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
    if (command === "ask_list_pending") {
      listCalls += 1;
      return Promise.resolve(listCalls === 1 ? [expired] : []);
    }
    if (command === "ask_expire") {
      return Promise.resolve({ expired: 1, nowMs: Date.now() });
    }
    return Promise.resolve(null);
  });
}

describe("ask store — local expiry guard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("C2-1/C2-2: cancel never submits an already-expired ask and reconciles via ask_expire", async () => {
    // Arrange: model the ≤2.5s window in which a stale card is still on screen. The snapshot is
    // injected directly — going through `refresh()` would let the C2-6 auto-reconcile clear the
    // card before the click even happens (fixture conflict, Lead ruling §9 ①).
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      // After the click the host has already transitioned the request: the re-list is empty.
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      if (command === "ask_expire") {
        return Promise.resolve({ expired: 1, nowMs: Date.now() });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    store.pendingAsks = [expired];
    store.lastRefreshMs = Date.now();
    expect(store.pendingAsks.map((ask) => ask.requestId)).toEqual(["ask-1"]);

    // Act
    await store.cancel(expired);

    // Assert (C2-1): the doomed CAS call must never leave the frontend.
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("ask_cancel", expect.anything());

    // Assert (C2-2): reconciliation is `ask_expire` followed by an `ask_list_pending` refresh.
    const sent = commands();
    const expireIndex = sent.indexOf("ask_expire");
    const lastListIndex = sent.lastIndexOf("ask_list_pending");
    expect(expireIndex).toBeGreaterThan(-1);
    expect(lastListIndex).toBeGreaterThan(expireIndex);
    expect(store.pendingAsks).toEqual([]);

    // Assert (C2-2): the user gets a Chinese message naming the expiry, not the bare CAS text.
    expect(store.error).toContain("过期");
    expect(String(store.error)).not.toContain("cannot be consumed");
  });

  it("C2-1/C2-2: answer never submits an already-expired ask and reconciles via ask_expire", async () => {
    // Arrange: same stale-card window as the cancel leg, injected directly (Lead ruling §9 ④).
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      if (command === "ask_expire") {
        return Promise.resolve({ expired: 1, nowMs: Date.now() });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    store.pendingAsks = [expired];
    store.lastRefreshMs = Date.now();

    // Act
    await store.answer(expired, "x");

    // Assert (C2-1)
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("ask_answer", expect.anything());
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("ask_cancel", expect.anything());

    // Assert (C2-2)
    const sent = commands();
    const expireIndex = sent.indexOf("ask_expire");
    const lastListIndex = sent.lastIndexOf("ask_list_pending");
    expect(expireIndex).toBeGreaterThan(-1);
    expect(lastListIndex).toBeGreaterThan(expireIndex);
    expect(store.pendingAsks).toEqual([]);
    expect(store.error).toContain("过期");
  });

  it("C2-1: a zero/absent TTL is treated as non-expiring, not as expired", async () => {
    // Arrange: `expiresAtMs <= 0` means "no TTL" in the host projection.
    const noTtl = createPendingAsk({ requestId: "ask-1", version: 4, expiresAtMs: 0 });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([noTtl]);
      }
      if (command === "ask_cancel") {
        return Promise.resolve({ requestId: "ask-1" });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    await store.refresh();

    // Act
    await store.cancel(noTtl);

    // Assert: no-TTL requests keep the normal CAS submission path.
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_cancel", {
      requestId: "ask-1",
      expectedVersion: 4,
      version: 4
    });
    expect(commands()).not.toContain("ask_expire");
  });

  it("C2-3: a live ask still cancels and answers with the version observed at listing time", async () => {
    // Arrange
    const live = createPendingAsk({
      requestId: "ask-1",
      version: 3,
      expiresAtMs: Date.now() + 60_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([live]);
      }
      if (command === "ask_cancel") {
        return Promise.resolve({ requestId: "ask-1" });
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-1", answer: "x" });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    await store.refresh();

    // Act
    await store.cancel(live);

    // Assert
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_cancel", {
      requestId: "ask-1",
      expectedVersion: 3,
      version: 3
    });
    expect(commands()).not.toContain("ask_expire");

    // Act 2: a second live ask goes through `ask_answer` with the same CAS shape.
    const liveAnswer = createPendingAsk({
      requestId: "ask-2",
      version: 5,
      expiresAtMs: Date.now() + 60_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([liveAnswer]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-2", answer: "x" });
      }
      return Promise.resolve(null);
    });
    await store.refresh();

    // Act
    await store.answer(liveAnswer, "x");

    // Assert
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_answer", {
      requestId: "ask-2",
      expectedVersion: 5,
      version: 5,
      answer: "x"
    });
    expect(commands()).not.toContain("ask_expire");
  });

  it("C2-3: an ask expiring only later is not treated as expired at submit time", async () => {
    // Arrange: a comfortable margin so a busy CI/desktop load cannot cross the TTL mid-test
    // (the previous `Date.now() + 1` fixture was flaky by construction; wave-16 修订 2).
    const almost = createPendingAsk({
      requestId: "ask-1",
      version: 7,
      expiresAtMs: Date.now() + 60_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([almost]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-1", answer: "是" });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    await store.refresh();

    // Act
    await store.answer(almost, "是");

    // Assert
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_answer", {
      requestId: "ask-1",
      expectedVersion: 7,
      version: 7,
      answer: "是"
    });
  });

  it("C2-5: isAskExpired mirrors the host boundary `expiresAtMs < now` exactly", () => {
    // Arrange: a fixed clock keeps the boundary assertions deterministic (no load flakiness).
    const now = 1_700_000_000_000;

    // Act + Assert: `expiresAtMs === now` is still host-consumable
    // (`can_consume`: `now_ms <= expires_at_ms`), so it must NOT be reported as expired.
    expect(isAskExpired({ expiresAtMs: now }, now)).toBe(false);

    // One millisecond past the boundary is genuinely expired.
    expect(isAskExpired({ expiresAtMs: now - 1 }, now)).toBe(true);
    expect(isAskExpired({ expiresAtMs: now }, now + 1)).toBe(true);

    // A long-lived / just-listed request is live.
    expect(isAskExpired({ expiresAtMs: now + 1 }, now)).toBe(false);

    // `expiresAtMs <= 0` means "no TTL", never "expired".
    expect(isAskExpired({ expiresAtMs: 0 }, now)).toBe(false);
    expect(isAskExpired({ expiresAtMs: -1 }, now)).toBe(false);
    expect(isAskExpired({ expiresAtMs: 0 }, Date.now())).toBe(false);

    // Degenerate inputs must not throw and must not be treated as expired.
    expect(isAskExpired(null, now)).toBe(false);
    expect(isAskExpired(undefined, now)).toBe(false);
  });

  it("C2-6: refresh auto-reconciles a locally expired ask (list → expire → list)", async () => {
    // Arrange: the panel disables every control of an expired card, so `answer`/`cancel` — the
    // only other `ask_expire` callers — are unreachable from the UI. `refresh` itself must
    // reconcile, otherwise the card and its `WaitingUser` run are stuck forever.
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    mockExpiredAskHost(expired);
    const store = useAskStore();

    // Act
    await store.refresh();

    // Assert (C2-6.1): exactly one expire + one re-list, in order, no recursion / no storm.
    expect(reconcileCommands()).toEqual(["ask_list_pending", "ask_expire", "ask_list_pending"]);

    // Assert (C2-6.2): the reconciled snapshot no longer carries the expired request.
    expect(store.pendingAsks).toEqual([]);

    // Assert (C2-6.1b, §7.1): a refresh-driven *successful* reconcile is silent — a residual
    // message would outlive the emptied list (polling self-terminates) and stick to the card.
    expect(store.error).toBeFalsy();
  });

  it("C2-6: a live snapshot produces no extra ask_expire IPC (zero side effects)", async () => {
    // Arrange: every poll tick lists a live ask; an unconditional expire would be a write storm.
    const live = createPendingAsk({
      requestId: "ask-1",
      version: 3,
      expiresAtMs: Date.now() + 60_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([live]);
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();

    // Act
    await store.refresh();

    // Assert
    expect(reconcileCommands()).toEqual(["ask_list_pending"]);
    expect(store.pendingAsks.map((ask) => ask.requestId)).toEqual(["ask-1"]);
  });

  it("C2-6: a failing ask_expire is surfaced and still re-lists the snapshot", async () => {
    // Arrange: the expire write fails (host unreachable); the failure must not be swallowed and
    // must not stop the re-list that is the whole point of the reconcile.
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    let listCalls = 0;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        listCalls += 1;
        return Promise.resolve(listCalls === 1 ? [expired] : []);
      }
      if (command === "ask_expire") {
        return Promise.reject(new Error("host unreachable"));
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();

    // Act
    await store.refresh();

    // Assert (C2-6.3): the failure is visible to the user…
    expect(store.error).toBeTruthy();

    // …and the re-list still ran, so the snapshot reflects host truth.
    expect(reconcileCommands()).toEqual(["ask_list_pending", "ask_expire", "ask_list_pending"]);
    expect(store.pendingAsks).toEqual([]);
  });

  // ── C2-7: the submit instant decides expiry, never the frozen listing snapshot ──────────────
  //
  // Scenario from review A finding 1: polling self-terminates on an empty list, so a later Ask is
  // listed exclusively by a `turn:suspended` refresh. `lastRefreshMs` freezes at that instant T;
  // a user who then waits past the TTL submits a request that is expired by the wall clock while
  // `expiresAtMs < T` is still false. Judging by the snapshot therefore leaks a doomed CAS.

  it("C2-7: cancel after the TTL must use the submit-time clock, not the listing snapshot", async () => {
    // Arrange
    vi.useFakeTimers();
    const ask = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() + 5_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_expire") {
        return Promise.resolve({ expired: 1, nowMs: Date.now() });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    await store.refresh();
    expect(store.lastRefreshMs).toBe(Date.now());

    // Act: the wall clock crosses the TTL without any new list snapshot.
    vi.advanceTimersByTime(6_000);
    expect(Date.now()).toBeGreaterThan(ask.expiresAtMs);
    await store.cancel(ask);

    // Assert: no doomed CAS, an expire reconcile instead, and an explicit expiry hint.
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("ask_cancel", expect.anything());
    expect(commands()).toContain("ask_expire");
    expect(store.error).toContain("过期");

    store.stopPolling();
  });

  it("C2-7: answer after the TTL must use the submit-time clock, not the listing snapshot", async () => {
    // Arrange
    vi.useFakeTimers();
    const ask = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() + 5_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_expire") {
        return Promise.resolve({ expired: 1, nowMs: Date.now() });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    await store.refresh();

    // Act
    vi.advanceTimersByTime(6_000);
    await store.answer(ask, "x");

    // Assert
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("ask_answer", expect.anything());
    expect(commands()).toContain("ask_expire");
    expect(store.error).toContain("过期");
  });

  // ── C2-8: a non-empty refresh must restart the self-terminated poll ─────────────────────────

  it("C2-8: refresh restarts polling when the snapshot becomes non-empty again", async () => {
    // Arrange: the first poll tick lists nothing (the run is idle), so polling self-terminates;
    // a `turn:suspended`-driven refresh then lists a live Ask.
    vi.useFakeTimers();
    const live = createPendingAsk({ requestId: "ask-1", expiresAtMs: Date.now() + 60_000 });
    let listCalls = 0;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        listCalls += 1;
        return Promise.resolve(listCalls === 1 ? [] : [live]);
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    store.startPolling();
    await vi.advanceTimersByTimeAsync(ASK_POLL_INTERVAL_MS);
    expect(store.polling).toBe(false);
    expect(store.pendingAsks).toEqual([]);

    // Act
    await store.refresh();
    expect(store.pendingAsks.map((ask) => ask.requestId)).toEqual(["ask-1"]);

    // Assert (C2-8): the poll must have been re-armed and must actually tick a new list call.
    expect(store.polling).toBe(true);
    const listsBefore = commands().filter((command) => command === "ask_list_pending").length;
    await vi.advanceTimersByTimeAsync(ASK_POLL_INTERVAL_MS);
    const listsAfter = commands().filter((command) => command === "ask_list_pending").length;
    expect(listsAfter).toBeGreaterThan(listsBefore);

    store.stopPolling();
  });

  // ── C2-9: single flight + optimistic-state protection ──────────────────────────────────────

  it("C2-9: two overlapping cancels of one expired ask issue exactly one ask_expire", async () => {
    // Arrange
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    mockExpiredAskHost(expired);
    const store = useAskStore();
    store.pendingAsks = [expired];
    store.lastRefreshMs = Date.now();
    tauriMocks.mockSafeInvoke.mockClear();

    // Act: double click — both calls start before either completes.
    const first = store.cancel(expired);
    const second = store.cancel(expired);
    await Promise.all([first, second]);

    // Assert (C2-9): one reconcile, never two; and never a doomed CAS.
    const expireCalls = commands().filter((command) => command === "ask_expire").length;
    expect(expireCalls).toBe(1);
    expect(commands()).not.toContain("ask_cancel");
    expect(store.error).toContain("过期");
  });

  it("C2-9b: a reconcile re-list must not re-populate an optimistically removed ask", async () => {
    // Arrange: A is live and being answered (optimistically removed); B is expired and drives the
    // reconcile. The snapshot is injected directly — `await refresh()` would block on the deferred
    // `ask_expire` (Lead ruling §9 ③); the deferreds keep the interleaving fully deterministic.
    const askA = createPendingAsk({
      requestId: "A",
      version: 1,
      expiresAtMs: Date.now() + 60_000
    });
    const askB = createPendingAsk({
      requestId: "B",
      version: 1,
      expiresAtMs: Date.now() - 1_000
    });

    let resolveExpire: (value: unknown) => void = () => {};
    const expirePromise = new Promise((resolve) => {
      resolveExpire = resolve;
    });
    let resolveAnswer: (value: unknown) => void = () => {};
    const answerPromise = new Promise((resolve) => {
      resolveAnswer = resolve;
    });

    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        // The reconcile re-list still reports the (host-side) live A.
        return Promise.resolve([askA]);
      }
      if (command === "ask_expire") {
        return expirePromise;
      }
      if (command === "ask_answer") {
        return answerPromise;
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    store.pendingAsks = [askA, askB];
    store.lastRefreshMs = Date.now();
    expect(store.pendingAsks.map((ask) => ask.requestId)).toEqual(["A", "B"]);

    // Act: answer(A) optimistically removes A and marks it busy while `ask_answer` is in flight;
    // cancel(B) drives the reconcile and blocks on the deferred `ask_expire`.
    const answerA = store.answer(askA, "x");
    expect(store.answeringRequestId).toBe("A");
    expect(store.pendingAsks.map((ask) => ask.requestId)).toEqual(["B"]);
    const cancelB = store.cancel(askB);
    expect(commands()).toContain("ask_expire");

    // The host finishes expiring; the reconcile re-lists [A].
    resolveExpire({ expired: 1, nowMs: Date.now() });
    await cancelB;

    // Assert: the in-flight answer's optimistic removal must survive the re-list.
    expect(store.pendingAsks.map((ask) => ask.requestId)).not.toContain("A");
    expect(store.pendingAsks).toEqual([]);

    // Teardown: let the suspended answer settle so no promise is left dangling.
    resolveAnswer({ requestId: "A", answer: "x" });
    await answerA;
  });
});
