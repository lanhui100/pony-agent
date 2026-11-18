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
  await vi.waitFor(() => expect(listeners.size).toBe(3));
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

  it("subscribes to changed, round and cleared with the frozen event names", async () => {
    const wrapper = await mountGoalBar();
    expect([...listeners.keys()].sort()).toEqual([
      "goal://changed",
      "goal://cleared",
      "goal://round"
    ]);
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
    expect(listeners.size).toBe(3);
    wrapper.unmount();
    expect(listeners.size).toBe(0);
  });
});
