import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import * as askStoreModule from "@/stores/ask";
import { ASK_POLL_INTERVAL_MS, ASK_STALE_MESSAGE, useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

// 波次 T3 / ADR 0074 重冻（取代 Wave-15 C2-1 ~ C2-9 的「本地过期判定」语义）：
//
// 旧契约（已废止）：本地时钟越过 `expiresAtMs` ⇒ 不提交 CAS、改走 `ask_expire` 对账、显示「过期」。
// 新契约（`.dev-team/contract-matrix-ask-persistence.md` §4/§5/§6、ADR 0074）：
//   1. 墙钟 TTL **退出产品语义**：客户端不得用本地时钟预判可否提交，也不得导出本地过期判定面；
//   2. 是否可作答只由宿主生命周期决定（宿主只列出 `Pending`）；陈旧 CAS 被拒 ⇒ 重新 list +
//      统一中文提示 `ASK_STALE_MESSAGE`，禁止上屏宿主英文内部串；
//   3. `ask_expire` 退化为 **GC 房管**（归档会话 `Orphaned` / 保留期），绝不再把 `Pending` 变死；
//   4. 显式忽略走 `ask_cancel`（生命周期 `Cancelled`），与「新用户消息 Superseded」区分。
//
// 断言纪律：全部落在 SUT 可观察面（IPC 调用序列、store 快照、用户可见文案）上。

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

function gatedCommands(): string[] {
  return commands().filter((command) =>
    ["ask_list_pending", "ask_expire", "ask_answer", "ask_cancel"].includes(command)
  );
}

describe("ask store — 无墙钟过期判定（ADR 0074 重冻）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("C2-1（重冻）：expiresAtMs 早已过去也必须提交 ask_cancel（时间不参与判定）", async () => {
    // Arrange: 旧语义下这张卡是「本地已过期」——必须不提交 CAS 并改走 ask_expire。
    const stale = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 60_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      if (command === "ask_cancel") {
        return Promise.resolve({ requestId: "ask-1" });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    store.pendingAsks = [stale];

    // Act
    await store.cancel(stale);

    // Assert: 正常 CAS 提交（用列表 version）。
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_cancel", {
      requestId: "ask-1",
      expectedVersion: 2,
      version: 2
    });

    // Assert: 不得出现任何「过期对账」调用，也不得出现「过期」文案。
    expect(commands()).not.toContain("ask_expire");
    expect(String(store.error ?? "")).not.toContain("过期");
  });

  it("C2-1（重冻）：expiresAtMs 早已过去也必须提交 ask_answer", async () => {
    // Arrange
    const stale = createPendingAsk({
      requestId: "ask-1",
      version: 4,
      expiresAtMs: 1_000
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ requestId: "ask-1" });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    store.pendingAsks = [stale];

    // Act
    await store.answer(stale, "是");

    // Assert
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_answer", {
      requestId: "ask-1",
      expectedVersion: 4,
      version: 4,
      answer: "是"
    });
    expect(commands()).not.toContain("ask_expire");
    expect(String(store.error ?? "")).not.toContain("过期");
  });

  it("C2-1（重冻）：expiresAtMs = 0 / 缺失同样走正常提交路径（无 TTL 分支）", async () => {
    // Arrange: 旧语义下 `<= 0` 是「无 TTL」特例；新语义下**所有**取值都是同一分支。
    const noTtl = createPendingAsk({ requestId: "ask-1", version: 4, expiresAtMs: 0 });
    const missing = createPendingAsk({ requestId: "ask-2", version: 5 });
    delete (missing as { expiresAtMs?: number }).expiresAtMs;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([noTtl, missing]);
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
    await store.cancel(missing);

    // Assert: 两次都是 CAS 提交，版本取列表快照。
    const cancelCalls = tauriMocks.mockSafeInvoke.mock.calls.filter(
      ([command]) => String(command) === "ask_cancel"
    );
    expect(cancelCalls.map(([, payload]) => payload)).toEqual([
      { requestId: "ask-1", expectedVersion: 4, version: 4 },
      { requestId: "ask-2", expectedVersion: 5, version: 5 }
    ]);
    expect(commands()).not.toContain("ask_expire");
  });

  it("C2-5（重冻）：store 不得再导出本地过期判定面 isAskExpired", () => {
    // 契约 §4：TTL 不再是拒绝理由，客户端也不允许保留本地墙钟判死面（否则会再次产生
    // 「本地判定 vs 宿主真相」两套语义）。
    expect(Object.keys(askStoreModule)).not.toContain("isAskExpired");
    expect(typeof (askStoreModule as Record<string, unknown>).isAskExpired).toBe("undefined");
  });

  it("C2-6（重冻）：refresh 不得自动 ask_expire 对账（列表即宿主真相）", async () => {
    // Arrange: 宿主已经不再返回该请求（已被忽略/取代/作答）。
    const stale = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000
    });
    let listCalls = 0;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        listCalls += 1;
        return Promise.resolve(listCalls === 1 ? [stale] : []);
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();

    // Act
    await store.refresh();
    expect(store.pendingAsks.map((ask) => ask.requestId)).toEqual(["ask-1"]);
    await store.refresh();

    // Assert: 只有 list，没有 expire；卡片随宿主列表消失。
    expect(gatedCommands()).toEqual(["ask_list_pending", "ask_list_pending"]);
    expect(store.pendingAsks).toEqual([]);
    expect(store.error).toBeFalsy();
  });

  it("C2-6（重冻）：显式 expire() 是 GC 房管——失败要可见且仍重新 list", async () => {
    // Arrange: 归档会话 GC 失败（宿主不可达）。
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      if (command === "ask_expire") {
        return Promise.reject(new Error("host unreachable"));
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();

    // Act
    const result = await store.expire();

    // Assert: 失败可见、返回 null，且不得伪造一次「成功对账」的本地状态。
    expect(result).toBeNull();
    expect(store.error).toBeTruthy();
    expect(gatedCommands()).toEqual(["ask_expire"]);
    expect(store.pendingAsks).toEqual([]);
    // 「清理」语义而不是「过期」语义：文案不得把 Pending 说成过期。
    expect(String(store.error)).not.toContain("过期");
  });

  it("C2-7（重冻）：墙钟越过旧 TTL 后 cancel 仍必须提交 CAS（提交时刻不判死）", async () => {
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
      if (command === "ask_cancel") {
        return Promise.resolve({ requestId: "ask-1" });
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    await store.refresh();
    expect(store.lastRefreshMs).toBe(Date.now());

    // Act: 墙钟越过旧 TTL（不产生新的列表快照）。
    vi.advanceTimersByTime(6_000);
    expect(Date.now()).toBeGreaterThan(ask.expiresAtMs);
    await store.cancel(ask);

    // Assert: 仍然提交 ask_cancel（旧语义此处必须“不提交 + expire 对账”）。
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_cancel", {
      requestId: "ask-1",
      expectedVersion: 2,
      version: 2
    });
    expect(commands()).not.toContain("ask_expire");
    expect(String(store.error ?? "")).not.toContain("过期");

    store.stopPolling();
  });

  it("C2-9（重冻）：双击忽略只发一次 ask_cancel（单飞），不得双提交", async () => {
    // Arrange
    const ask = createPendingAsk({ requestId: "ask-1", version: 2 });
    let resolveCancel: (value: unknown) => void = () => {};
    const cancelPromise = new Promise((resolve) => {
      resolveCancel = resolve;
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_cancel") {
        return cancelPromise;
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    store.pendingAsks = [ask];

    // Act: 双击——两次调用都在第一次完成前发起。
    const first = store.ignore(ask);
    const second = store.ignore(ask);
    resolveCancel({ requestId: "ask-1" });
    const results = await Promise.all([first, second]);

    // Assert: 只有一次真实 CAS 提交，第二次被单飞挡下（返回 null）。
    const cancelCalls = commands().filter((command) => command === "ask_cancel");
    expect(cancelCalls).toHaveLength(1);
    expect(results.filter((value) => value != null)).toHaveLength(1);
    expect(store.pendingAsks).toEqual([]);
  });

  it("C2-9b（重冻）：陈旧 CAS 被拒 ⇒ 重新 list + 统一中文失效提示（不上屏宿主英文）", async () => {
    // Arrange: 宿主已把该请求交出（作答/取代/忽略），CAS 返回典型陈旧错误。
    const ask = createPendingAsk({ requestId: "ask-1", version: 2 });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([]);
      }
      if (command === "ask_cancel") {
        return Promise.reject(new Error("control request `ask-1` cannot be consumed: replay"));
      }
      return Promise.resolve(null);
    });
    const store = useAskStore();
    store.pendingAsks = [ask];

    // Act
    await store.ignore(ask);

    // Assert: 先重新 list（宿主真相），再给统一中文提示。
    expect(gatedCommands()).toEqual(["ask_cancel", "ask_list_pending"]);
    expect(store.pendingAsks).toEqual([]);
    expect(store.error).toContain(ASK_STALE_MESSAGE);
    expect(String(store.error)).not.toContain("cannot be consumed");
    expect(String(store.error)).not.toContain("过期");
  });

  it("C2-8：refresh 在快照重新非空时必须重启自停的轮询（与 TTL 无关，保留）", async () => {
    // Arrange
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

    // Assert
    expect(store.polling).toBe(true);
    const listsBefore = commands().filter((command) => command === "ask_list_pending").length;
    await vi.advanceTimersByTimeAsync(ASK_POLL_INTERVAL_MS);
    const listsAfter = commands().filter((command) => command === "ask_list_pending").length;
    expect(listsAfter).toBeGreaterThan(listsBefore);

    store.stopPolling();
  });
});
