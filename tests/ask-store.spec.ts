import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { isStaleAskError, useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

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
    descriptorId: partial.descriptorId ?? "builtin:ask",
    finalArgsDigest: partial.finalArgsDigest ?? "args-digest",
    policyDigest: partial.policyDigest ?? "policy-digest",
    nonce: partial.nonce ?? "nonce-1",
    version: partial.version ?? 1,
    expiresAtMs: partial.expiresAtMs ?? Date.now() + 60_000,
    state: partial.state ?? "pending",
    prompt: partial.prompt ?? "continue?",
    options: partial.options ?? null
  };
}

describe("ask store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("lists pending asks through ask_list_pending", async () => {
    const asks = [createPendingAsk({ requestId: "ask-1" }), createPendingAsk({ requestId: "ask-2" })];
    tauriMocks.mockSafeInvoke.mockResolvedValue(asks);

    const store = useAskStore();
    const result = await store.refresh();

    expect(result).toHaveLength(2);
    expect(store.pendingAsks).toHaveLength(2);
    expect(store.pendingCount).toBe(2);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_list_pending", {});
  });

  it("filters by sessionId when provided", async () => {
    tauriMocks.mockSafeInvoke.mockResolvedValue([]);

    const store = useAskStore();
    await store.refresh({ sessionId: "session-1" });

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_list_pending", {
      sessionId: "session-1"
    });
  });

  it("falls back to an empty list when the host returns null", async () => {
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);

    const store = useAskStore();
    const result = await store.refresh();

    expect(result).toEqual([]);
    expect(store.pendingAsks).toEqual([]);
  });

  it("keeps the empty list in browser preview mode", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    const store = useAskStore();
    store.pendingAsks = [createPendingAsk()];
    const result = await store.refresh();

    expect(result).toEqual([]);
    expect(store.pendingAsks).toEqual([]);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalled();
  });

  it("answers with the listed version and removes the ask locally without a re-fetch", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", version: 3 });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-1", answer: "yes" });
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    await store.refresh();

    const result = await store.answer(ask, "yes");

    expect(result).toEqual({ requestId: "ask-1", answer: "yes" });
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_answer", {
      requestId: "ask-1",
      expectedVersion: 3,
      answer: "yes"
    });
    // The request is dropped from the snapshot without calling ask_list_pending again.
    expect(store.pendingAsks).toEqual([]);
    expect(tauriMocks.mockSafeInvoke.mock.calls.filter(([command]) => command === "ask_list_pending")).toHaveLength(1);
  });

  it("rejects a resubmit on a request that is no longer in the snapshot", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", version: 2 });
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce([ask]);

    const store = useAskStore();
    await store.refresh();

    // Simulate the ask having already been consumed by another consumer: the
    // snapshot no longer contains it, so the stale version must not be resubmitted.
    store.pendingAsks = [];

    const result = await store.answer(ask, "yes");

    expect(result).toBeNull();
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("ask_answer", expect.anything());
  });

  it("answers and calls graph_resume_ask to resume the bound graph run", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", runId: "run-1", version: 2 });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-1", answer: "yes" });
      }
      if (command === "graph_list_ask_waits") {
        return Promise.resolve([
          {
            requestId: "ask-1",
            expectedVersion: 2,
            runId: "run-1",
            turnId: "turn-1",
            callId: "call-1",
            toolName: "Ask",
            createdAtMs: 1_000
          }
        ]);
      }
      if (command === "graph_resume_ask") {
        return Promise.resolve({ runId: "run-1", callId: "call-1", answer: "yes" });
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    await store.refresh();

    const result = await store.answer(ask, "yes");

    expect(result).toEqual({ requestId: "ask-1", answer: "yes" });
    expect(store.pendingAsks).toEqual([]);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("graph_list_ask_waits", {
      runId: "run-1"
    });
    // The binding's expectedVersion is presented to graph_resume_ask, not the post-consumption
    // request version (graph resume is keyed to the binding version).
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("graph_resume_ask", {
      runId: "run-1",
      requestId: "ask-1",
      version: 2,
      answer: "yes"
    });
  });

  it("does not call graph_resume_ask when the ask has no run id", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", runId: "" });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-1" });
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    await store.refresh();
    await store.answer(ask, "yes");

    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("graph_list_ask_waits", expect.anything());
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("graph_resume_ask", expect.anything());
  });

  it("keeps the resume failure non-fatal when the graph wait is already consumed", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", runId: "run-1" });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-1", answer: "yes" });
      }
      if (command === "graph_list_ask_waits") {
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    await store.refresh();

    const result = await store.answer(ask, "yes");

    // The answer itself succeeded; the missing binding is not an error.
    expect(result).toEqual({ requestId: "ask-1", answer: "yes" });
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("graph_resume_ask", expect.anything());
    expect(store.error).toBeNull();
  });

  it("watches turn:suspended so a freshly paused run surfaces its ask", async () => {
    const store = useAskStore();
    await store.startWatchingEvents();

    const listenedEvents = tauriMocks.mockSafeListen.mock.calls.map(([event]) => event);
    expect(listenedEvents).toContain("turn:completed");
    expect(listenedEvents).toContain("turn:failed");
    expect(listenedEvents).toContain("turn:cancelled");
    expect(listenedEvents).toContain("turn:suspended");
  });

  it("refuses to answer a non-interaction request", async () => {
    const approval = createPendingAsk({ requestId: "approve-1", requestKind: "approval" });
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce([approval]);

    const store = useAskStore();
    await store.refresh();

    const result = await store.answer(approval, "yes");

    expect(result).toBeNull();
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("ask_answer", expect.anything());
  });

  it("disables concurrent answers while a request is being answered", async () => {
    const ask = createPendingAsk({ requestId: "ask-1" });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_answer") {
        return new Promise((resolve) => {
          setTimeout(() => resolve({ ok: true }), 10);
        });
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    await store.refresh();

    const first = store.answer(ask, "yes");
    const second = store.answer(ask, "no");

    expect(store.answeringRequestId).toBe("ask-1");
    expect(second).resolves.toBeNull();
    await first;
    expect(store.answeringRequestId).toBeNull();
  });

  it("reconciles the snapshot when a stale-version answer is rejected", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", version: 1 });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      if (command === "ask_answer") {
        return Promise.reject(new Error("stale mutation rejected: version mismatch"));
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    store.pendingAsks = [ask];

    const result = await store.answer(ask, "yes");

    expect(result).toBeNull();
    expect(store.error).toContain("应答失败");
    await vi.waitFor(() => {
      expect(tauriMocks.mockSafeInvoke.mock.calls.filter(([command]) => command === "ask_list_pending").length).toBeGreaterThanOrEqual(1);
    });
  });

  it("cancels a pending ask with the listed version and removes it locally", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", version: 5 });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_cancel") {
        return Promise.resolve({ requestId: "ask-1" });
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    await store.refresh();

    const result = await store.cancel(ask);

    expect(result).toEqual({ requestId: "ask-1" });
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_cancel", {
      requestId: "ask-1",
      expectedVersion: 5
    });
    expect(store.pendingAsks).toEqual([]);
    expect(store.cancellingRequestId).toBeNull();
  });

  it("calls ask_expire and reconciles the list", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_expire") {
        return Promise.resolve({ expired: 2, nowMs: 1_000 });
      }
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    const expired = await store.expire();

    expect(expired).toBe(2);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_expire");
    expect(store.pendingAsks).toEqual([]);
  });

  it("polls while pending asks remain and self-terminates when empty", async () => {
    vi.useFakeTimers();
    const ask = createPendingAsk({ requestId: "ask-1" });
    let listCallCount = 0;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        listCallCount += 1;
        // First refresh returns one ask; subsequent refreshes return none.
        return Promise.resolve(listCallCount === 1 ? [ask] : []);
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    await store.refresh();
    expect(store.pendingAsks).toHaveLength(1);

    store.startPolling(1000);
    expect(store.polling).toBe(true);

    await vi.advanceTimersByTimeAsync(1050);
    await vi.advanceTimersByTimeAsync(1050);
    await vi.advanceTimersByTimeAsync(1050);

    // Second refresh (listCallCount 2) returned empty and stopped the poll.
    expect(store.polling).toBe(false);
    expect(store.pendingAsks).toEqual([]);

    vi.useRealTimers();
  });

  it("detects host CAS failure messages as stale", () => {
    expect(isStaleAskError("control request `ask-1` cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch")).toBe(true);
    expect(isStaleAskError("Ask `ask-1` expected version 1 but current version is 2; stale mutation rejected.")).toBe(true);
    expect(isStaleAskError("unknown control request `ask-1`")).toBe(true);
    expect(isStaleAskError("control request `ask-1` kind mismatch")).toBe(true);
    expect(isStaleAskError("network down")).toBe(false);
  });
});
