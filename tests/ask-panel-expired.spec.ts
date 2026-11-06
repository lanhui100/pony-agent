import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import AskPanel from "@/components/AskPanel.vue";
import { useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

// 波次 T3 / ADR 0074 重冻（取代 Wave-15 C2-4「过期禁用 + 已过期文案」）：
//
// 旧契约（已废止）：本地时钟越过 `expiresAtMs` ⇒ 面板禁用全部提交控件并显示「过期」。
// 新契约（`.dev-team/contract-matrix-ask-persistence.md` §6 / ADR 0074）：
//   1. **无倒计时、无「已过期」态**；控件不得因本地时钟而禁用（是否可作答由宿主生命周期判定，
//      宿主只列出 `Pending`）；
//   2. **恢复即重建**：挂载/打开会话必须重新 list 待决 Ask；
//   3. **显式忽略**：`ask-ignore-submit` ⇒ `ask_cancel`（生命周期 `Cancelled`）⇒ 卡片消失且不再出现。
//
// 断言纪律：全部落在组件可观察形态（DOM 属性/文案/IPC 调用）上，不依赖内部实现细节。

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
    prompt: partial.prompt ?? "继续执行？",
    options: partial.options ?? ["是", "否"]
  };
}

async function flushAsync() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

async function mountWithAsk(ask: PendingAsk) {
  tauriMocks.mockSafeInvoke.mockResolvedValue([ask]);
  const wrapper = mount(AskPanel);
  await flushAsync();
  await vi.waitFor(() => {
    expect(useAskStore().pendingAsks).toHaveLength(1);
  });
  await wrapper.vm.$nextTick();
  return wrapper;
}

/**
 * 在不触碰（已被 faked 的）定时器队列的前提下冲净微任务：`vi.useFakeTimers()` 生效时
 * `setTimeout(resolve, 0)` 永远不会触发，必须改用纯微任务等待。
 */
async function flushMicrotasks(rounds = 12) {
  for (let round = 0; round < rounds; round += 1) {
    await Promise.resolve();
  }
}

function commands(): string[] {
  return tauriMocks.mockSafeInvoke.mock.calls.map(([command]) => String(command));
}

describe("AskPanel — 无墙钟过期态 + 显式忽略（ADR 0074）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("C2-4（重冻）：expiresAtMs 早已过去也不得禁用任何提交控件", async () => {
    // Arrange: 旧语义下这张卡是「已过期」——必须被禁用并显示「过期」。
    const stale = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 60_000,
      options: ["是", "否"]
    });

    // Act
    const wrapper = await mountWithAsk(stale);

    // Assert: 控件全部可用（是否可作答由宿主 CAS 判定，不由本地时钟判定）。
    const optionButtons = wrapper.findAll('[data-testid="ask-options"] button');
    expect(optionButtons).toHaveLength(2);
    for (const button of optionButtons) {
      expect(button.attributes("disabled")).toBeUndefined();
    }
    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeUndefined();
    expect(wrapper.get('[data-testid="ask-ignore-submit"]').attributes("disabled")).toBeUndefined();

    // Assert: 不得出现「过期」态文案（无已过期提示、无倒计时）。
    expect(wrapper.text()).not.toContain("过期");
    expect(wrapper.find('[data-testid="ask-expired"]').exists()).toBe(false);

    wrapper.unmount();
  });

  it("C2-4（control）：无论创建/到期时间如何，活着的卡片同样可用（无 TTL 分支）", async () => {
    // Arrange: 与上一条构成对照——两条卡片的控件状态必须一致，说明「时间」根本不是分支条件。
    const live = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() + 60_000,
      options: ["是", "否"]
    });

    // Act
    const wrapper = await mountWithAsk(live);

    // Assert
    const optionButtons = wrapper.findAll('[data-testid="ask-options"] button');
    expect(optionButtons).toHaveLength(2);
    for (const button of optionButtons) {
      expect(button.attributes("disabled")).toBeUndefined();
    }
    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeUndefined();
    expect(wrapper.get('[data-testid="ask-ignore-submit"]').attributes("disabled")).toBeUndefined();

    wrapper.unmount();
  });

  it("C2-4（重冻）：无 options 的自由文本 Ask 同样不受时间影响", async () => {
    // Arrange
    const stale = createPendingAsk({
      requestId: "ask-1",
      expiresAtMs: Date.now() - 1,
      options: []
    });

    // Act
    const wrapper = await mountWithAsk(stale);

    // Assert
    expect(wrapper.find('[data-testid="ask-options"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeUndefined();
    expect(wrapper.get('[data-testid="ask-ignore-submit"]').attributes("disabled")).toBeUndefined();
    expect(wrapper.text()).not.toContain("过期");

    wrapper.unmount();
  });

  it("C2-10（重冻）：墙钟推进 + 强制重渲染后仍不得禁用（渲染不读本地时钟判活）", async () => {
    // Arrange
    vi.useFakeTimers();
    const ask = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() + 5_000,
      options: ["是", "否"]
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      return Promise.resolve(null);
    });

    const store = useAskStore();
    store.pendingAsks = [ask];
    store.lastRefreshMs = Date.now();
    const wrapper = mount(AskPanel);
    await flushMicrotasks();
    await wrapper.vm.$nextTick();

    expect(store.pendingAsks.map((pending) => pending.requestId)).toEqual(["ask-1"]);

    // Act: 只推进墙钟（不触发任何定时器），再强制重渲染。
    vi.setSystemTime(new Date(Date.now() + 6_000));
    wrapper.vm.$forceUpdate();
    await wrapper.vm.$nextTick();

    // Assert: 越过旧 TTL 之后控件必须**依然可用**，且没有任何「过期」文案。
    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeUndefined();
    expect(wrapper.get('[data-testid="ask-ignore-submit"]').attributes("disabled")).toBeUndefined();
    for (const button of wrapper.findAll('[data-testid="ask-options"] button')) {
      expect(button.attributes("disabled")).toBeUndefined();
    }
    expect(wrapper.text()).not.toContain("过期");

    wrapper.unmount();
    store.stopPolling();
  });

  it("C2-4（重冻）：卡片展示提问时间（createdAtMs）而不是截止倒计时", async () => {
    // Arrange: createdAtMs = 固定时刻，避免断言依赖当前时间。
    const createdAtMs = new Date(2026, 10, 6, 9, 30).getTime();
    const ask = createPendingAsk({ requestId: "ask-1", version: 1 });
    (ask as { createdAtMs?: number }).createdAtMs = createdAtMs;

    // Act
    const wrapper = await mountWithAsk(ask);

    // Assert: 有「提问于」时间位，且该时间位用等宽数字（tabular-nums）；无倒计时位。
    const createdAt = wrapper.get('[data-testid="ask-created-at"]');
    expect(createdAt.text()).toContain("提问于");
    expect(createdAt.classes()).toContain("tabular-nums");
    expect(wrapper.find('[data-testid="ask-expiry-countdown"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("剩余");

    wrapper.unmount();
  });

  it("C2-4（重冻·契约 §6.3）：点击「忽略此提问」⇒ ask_cancel（Cancelled）且卡片消失不再出现", async () => {
    // Arrange: 忽略是**显式生命周期动作**（Cancelled），不是过期，也不是本地丢弃。
    const ask = createPendingAsk({ requestId: "ask-1", version: 3, options: ["是", "否"] });
    let listCalls = 0;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        listCalls += 1;
        return Promise.resolve(listCalls === 1 ? [ask] : []);
      }
      if (command === "ask_cancel") {
        return Promise.resolve({ requestId: "ask-1", lifecycleState: "cancelled" });
      }
      return Promise.resolve(null);
    });

    const wrapper = mount(AskPanel);
    await flushAsync();
    const store = useAskStore();
    await vi.waitFor(() => {
      expect(store.pendingAsks).toHaveLength(1);
    });

    // Act
    await wrapper.get('[data-testid="ask-ignore-submit"]').trigger("click");
    await flushAsync();

    // Assert: 走的是 ask_cancel + 列表里的 version（CAS），不是 ask_expire 对账。
    const cancelCall = tauriMocks.mockSafeInvoke.mock.calls.find(
      ([command]) => String(command) === "ask_cancel"
    );
    expect(cancelCall).toBeTruthy();
    expect(cancelCall?.[1]).toEqual({
      requestId: "ask-1",
      expectedVersion: 3,
      version: 3
    });
    expect(commands()).not.toContain("ask_expire");

    // Assert: 卡片消失，且再次 list 也不会把它带回来（宿主已置 Cancelled）。
    expect(store.pendingAsks).toHaveLength(0);
    await store.refresh();
    expect(store.pendingAsks).toHaveLength(0);
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[data-testid="ask-card"]').exists()).toBe(false);

    wrapper.unmount();
  });
});
