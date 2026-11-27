import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import GoalBar from "@/components/GoalBar.vue";

// O7（2026-11-27）：goal 的**双预算**必须在桌面端分别展示。
//
// 要防的回归（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` P2-1）：界面只展示目标预算
// （50 轮），用户看到 8 轮就停，观感是「承诺 50 实跑 8」，且没有任何界面解释这 8 轮的来源。
//
// 契约：文案真源在 Rust `pony_storage::goal::budget_summary_lines`（与 TUI `/goal show` 同源），
// 前端经 additive 命令 `goal_budget_lines` 取回两行**原样**渲染，不得自行拼措辞。

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

const SAMPLE_GOAL = {
  id: "goal-1",
  sessionId: "session-1",
  objective: "把双预算讲清楚",
  phase: "active",
  activation: "armed",
  revision: 1,
  roundsStarted: 3,
  maxGoalRounds: 50,
  blockedReason: null
};

const SAMPLE_BUDGET = {
  perInputLine: "自动续轮: 每次输入最多 8 轮（之后等你发话，目标保持激活）",
  targetBudgetLine: "目标预算: 3/50 轮（跨多次输入累计；用满后目标转 blocked(round_limit)）"
};

async function mountGoalBar(sessionId = "session-1") {
  const wrapper = mount(GoalBar, { props: { sessionId, open: true } });
  await vi.waitFor(() => expect(listeners.size).toBe(4));
  await flushPromises();
  return wrapper;
}

describe("GoalBar dual budget (O7)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    listeners.clear();
    vi.clearAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "goal_get") return SAMPLE_GOAL;
      if (command === "goal_budget_lines") return SAMPLE_BUDGET;
      return null;
    });
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

  it("requests budget lines for the current session on mount", async () => {
    const wrapper = await mountGoalBar();
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("goal_budget_lines", {
      sessionId: "session-1"
    });
    wrapper.unmount();
  });

  it("renders both budget lines verbatim from the host", async () => {
    const wrapper = await mountGoalBar();
    const block = wrapper.find("[data-testid='goal-budget-lines']");
    expect(block.exists()).toBe(true);
    // 逐字渲染：措辞由 Rust 提供，前端拼错即红。
    expect(block.text()).toContain(SAMPLE_BUDGET.perInputLine);
    expect(block.text()).toContain(SAMPLE_BUDGET.targetBudgetLine);
    wrapper.unmount();
  });

  it("keeps the target-budget line that already existed (both口径 must coexist)", async () => {
    const wrapper = await mountGoalBar();
    expect(wrapper.text()).toContain("轮次: 3 / 50");
    expect(wrapper.find("[data-testid='goal-budget-lines']").text()).toContain("目标预算: 3/50 轮");
    wrapper.unmount();
  });

  it("omits the budget block when the host reports no goal", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "goal_get") return SAMPLE_GOAL;
      if (command === "goal_budget_lines") return null;
      return null;
    });
    const wrapper = await mountGoalBar();
    expect(wrapper.find("[data-testid='goal-budget-lines']").exists()).toBe(false);
    wrapper.unmount();
  });

  it("does not render a malformed budget payload as an empty line", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "goal_get") return SAMPLE_GOAL;
      // 缺字段的载荷（模拟契约漂移）⇒ 宁可不渲染，也不要把 undefined 画成空行。
      if (command === "goal_budget_lines") return { perInputLine: "只有一行" };
      return null;
    });
    const wrapper = await mountGoalBar();
    expect(wrapper.find("[data-testid='goal-budget-lines']").exists()).toBe(false);
    wrapper.unmount();
  });

  it("survives a failing budget call without breaking the goal bar", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "goal_get") return SAMPLE_GOAL;
      if (command === "goal_budget_lines") throw new Error("host offline");
      return null;
    });
    const wrapper = await mountGoalBar();
    // 目标条本身仍然渲染（预算行是展示增强，不是渲染前置条件）。
    expect(wrapper.text()).toContain("把双预算讲清楚");
    expect(wrapper.find("[data-testid='goal-budget-lines']").exists()).toBe(false);
    wrapper.unmount();
  });

  it("refreshes budget lines when the session changes", async () => {
    const wrapper = await mountGoalBar();
    tauriMocks.mockSafeInvoke.mockClear();
    await wrapper.setProps({ sessionId: "session-2" });
    await flushPromises();
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("goal_budget_lines", {
      sessionId: "session-2"
    });
    wrapper.unmount();
  });
});
