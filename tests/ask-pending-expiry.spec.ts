import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAskStore } from "@/stores/ask";
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
    // Arrange
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    mockExpiredAskHost(expired);
    const store = useAskStore();
    await store.refresh();
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
    // Arrange
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    mockExpiredAskHost(expired);
    const store = useAskStore();
    await store.refresh();

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
    // Arrange: 1ms short of expiry is still live.
    const almost = createPendingAsk({
      requestId: "ask-1",
      version: 7,
      expiresAtMs: Date.now() + 1
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
});
