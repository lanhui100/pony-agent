import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import GoalBar from "@/components/GoalBar.vue";

// B130：GoalBar 订阅的三个宿主事件必须真的能驱动刷新。
//
// 旧实现订阅 `goal://changed` / `goal://round`，但 Rust 侧零发射点（死通道）；B130 在
// 领域层补齐发射（`src-tauri/src/goal_events.rs`）。本用例锁定前端这一半：
//   1. 三个事件名与后端常量一致；
//   2. `sessionId` 不匹配的 changed/cleared 不得触发跨会话刷新；
//   3. round 事件不看 sessionId（载荷本身不含 sessionId，与 §7 一致）。

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

type Listener = (event: { payload: unknown }) => void;

const listeners = new Map<string, Listener>();

function goalGetCalls(): number {
  return tauriMocks.mockSafeInvoke.mock.calls.filter(([command]) => command === "goal_get").length;
}

async function mountGoalBar(sessionId = "session-1") {
  const wrapper = mount(GoalBar, { props: { sessionId } });
  // onMounted 内的订阅是 async IIFE：等 safeListen 全部登记完成。
  // O6（2026-11-27）：新增 `goal://rounds_stopped` 订阅 ⇒ 四个通道。
  await vi.waitFor(() => expect(listeners.size).toBe(4));
  await flushPromises();
  return wrapper;
}

describe("GoalBar goal:// events (B130)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    listeners.clear();
    vi.clearAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    tauriMocks.mockSafeListen.mockImplementation(
      async (name: string, handler: Listener) => {
        listeners.set(name, handler);
        return () => listeners.delete(name);
      }
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("subscribes to changed, round, cleared and rounds_stopped with the frozen event names", async () => {
    const wrapper = await mountGoalBar();
    expect([...listeners.keys()].sort()).toEqual([
      "goal://changed",
      "goal://cleared",
      "goal://round",
      "goal://rounds_stopped"
    ]);
    wrapper.unmount();
  });

  // O6（2026-11-27）：续轮停止/暂停必须在桌面可见（审核报告 P1-3：桌面端此前零事件面）。
  it("renders the rounds_stopped visibleLine for the matching session and clears it on the next round", async () => {
    tauriMocks.mockSafeInvoke.mockResolvedValue({
      id: "goal-1",
      sessionId: "session-1",
      objective: "把审核报告 P0 落地",
      phase: "active",
      activation: "armed",
      revision: 1,
      roundsStarted: 2,
      maxGoalRounds: 50,
      blockedReason: null
    });
    const wrapper = await mountGoalBar();
    await flushPromises();

    const stopped = listeners.get("goal://rounds_stopped")!;
    stopped({ payload: { sessionId: "session-2", visibleLine: "别的会话" } });
    await flushPromises();
    expect(wrapper.find("[data-testid='goal-rounds-stopped']").exists()).toBe(false);

    stopped({
      payload: {
        sessionId: "session-1",
        goalId: "goal-1",
        reason: "turn_suspended",
        visibleLine: "目标已暂停：等你作答｜本轮在等你作答/审批｜先回答上面的提问，作答后自动继续",
        roundsStarted: 2,
        maxRoundRounds: 50,
        consecutiveFailures: 0
      }
    });
    await flushPromises();
    const line = wrapper.find("[data-testid='goal-rounds-stopped']");
    expect(line.exists()).toBe(true);
    expect(line.text()).toContain("等你作答");

    // 新的一轮起来 ⇒ 上一条停止原因不再成立。
    listeners.get("goal://round")!({ payload: { round: 3, status: "started" } });
    await flushPromises();
    expect(wrapper.find("[data-testid='goal-rounds-stopped']").exists()).toBe(false);
    wrapper.unmount();
  });

  it("refreshes on a matching goal://changed payload only", async () => {
    const wrapper = await mountGoalBar();
    const changed = listeners.get("goal://changed")!;

    tauriMocks.mockSafeInvoke.mockClear();
    changed({ payload: { sessionId: "session-2" } });
    await flushPromises();
    expect(goalGetCalls()).toBe(0);

    changed({ payload: { sessionId: "session-1" } });
    await flushPromises();
    expect(goalGetCalls()).toBe(1);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("goal_get", {
      sessionId: "session-1"
    });
    wrapper.unmount();
  });

  it("refreshes on goal://round without a session filter", async () => {
    const wrapper = await mountGoalBar();
    tauriMocks.mockSafeInvoke.mockClear();

    listeners.get("goal://round")!({ payload: { round: 4, status: "started" } });
    await flushPromises();

    expect(goalGetCalls()).toBe(1);
    wrapper.unmount();
  });

  it("refreshes on goal://cleared so a deleted goal disappears immediately", async () => {
    const wrapper = await mountGoalBar();
    const cleared = listeners.get("goal://cleared")!;

    tauriMocks.mockSafeInvoke.mockClear();
    cleared({ payload: { sessionId: "session-2", goalId: "goal-x" } });
    await flushPromises();
    expect(goalGetCalls()).toBe(0);

    cleared({ payload: { sessionId: "session-1", goalId: "goal-1" } });
    await flushPromises();
    expect(goalGetCalls()).toBe(1);
    wrapper.unmount();
  });

  it("unsubscribes every listener on unmount", async () => {
    const wrapper = await mountGoalBar();
    expect(listeners.size).toBe(4);
    wrapper.unmount();
    expect(listeners.size).toBe(0);
  });
});
