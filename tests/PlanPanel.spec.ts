import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import PlanPanel from "@/components/PlanPanel.vue";
import { useAskStore } from "@/stores/ask";
import { usePlanStore } from "@/stores/plan";
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
    prompt: partial.prompt ?? "继续执行?",
    options: partial.options ?? null
  };
}

async function flushAsync() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

describe("PlanPanel（Plan 模式 + 独立任务进度）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  const mockMode = (mode: Record<string, unknown>) => {
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "plan_mode_get") {
        return Promise.resolve({
          sessionId: "session-1",
          active: false,
          plan: null,
          planApproved: false,
          revision: 0,
          ...mode
        });
      }
      if (command === "todo_list") {
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });
  };

  it("读取 Plan 模式并渲染方案正文（不再有步骤清单）", async () => {
    mockMode({ active: true, plan: "# 接入面板\n- 梳理接入点", revision: 3 });

    const wrapper = mount(PlanPanel, { props: { sessionId: "session-1", open: true } });
    await flushAsync();
    await vi.waitFor(() => {
      expect(usePlanStore().mode.active).toBe(true);
    });

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_mode_get", {
      sessionId: "session-1"
    });
    expect(wrapper.get('[data-testid="plan-mode-card"]').text()).toContain("规划中");
    expect(wrapper.get('[data-testid="plan-mode-body"]').text()).toContain("接入面板");
    expect(wrapper.find('[data-testid="plan-steps"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("折叠头只显示任务 x/x（不与 Plan 混加）", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "plan_mode_get") {
        return Promise.resolve({
          sessionId: "session-1",
          active: true,
          plan: "# 方案",
          planApproved: false,
          revision: 1
        });
      }
      if (command === "todo_list") {
        return Promise.resolve([
          { content: "编写单元测试", status: "completed" },
          { content: "实现任务面板", status: "in_progress" },
          { content: "验证样式表现", status: "pending" }
        ]);
      }
      return Promise.resolve(null);
    });

    const wrapper = mount(PlanPanel, { props: { sessionId: "session-1", open: true } });
    await flushAsync();
    await vi.waitFor(() => {
      expect(usePlanStore().todos).toHaveLength(3);
    });

    expect(wrapper.get('[data-testid="plan-panel-task-progress"]').text()).toBe("1/3");
    expect(wrapper.get('[data-testid="plan-todos"]').text()).toContain("实现任务面板");
    wrapper.unmount();
  });

  it("待审方案可点「批准执行」，走 plan_mode_review 并退出模式", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "plan_mode_get") {
        return Promise.resolve({
          sessionId: "session-1",
          active: true,
          plan: "# 草稿方案",
          planApproved: false,
          revision: 1
        });
      }
      if (command === "plan_mode_review") {
        return Promise.resolve({
          sessionId: "session-1",
          active: false,
          plan: "# 草稿方案",
          planApproved: true,
          revision: 2
        });
      }
      if (command === "todo_list") {
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });

    const wrapper = mount(PlanPanel, { props: { sessionId: "session-1", open: true } });
    await flushAsync();
    const store = usePlanStore();
    await vi.waitFor(() => {
      expect(store.mode.active).toBe(true);
    });

    const approveBtn = wrapper.get('[data-testid="plan-approve-execution"]');
    expect(approveBtn.text()).toContain("批准执行");
    await approveBtn.trigger("click");
    await flushAsync();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_mode_review", {
      sessionId: "session-1",
      approved: true
    });
    expect(store.mode.active).toBe(false);
    expect(store.mode.planApproved).toBe(true);
    wrapper.unmount();
  });

  it("「继续规划」保持模式激活且方案未批准", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "plan_mode_get") {
        return Promise.resolve({
          sessionId: "session-1",
          active: true,
          plan: "# 草稿方案",
          planApproved: false,
          revision: 1
        });
      }
      if (command === "plan_mode_review") {
        return Promise.resolve({
          sessionId: "session-1",
          active: true,
          plan: "# 草稿方案",
          planApproved: false,
          revision: 2
        });
      }
      if (command === "todo_list") {
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });

    const wrapper = mount(PlanPanel, { props: { sessionId: "session-1", open: true } });
    await flushAsync();
    await vi.waitFor(() => {
      expect(usePlanStore().mode.active).toBe(true);
    });

    await wrapper.get('[data-testid="plan-keep-planning"]').trigger("click");
    await flushAsync();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_mode_review", {
      sessionId: "session-1",
      approved: false
    });
    expect(usePlanStore().mode.active).toBe(true);
    wrapper.unmount();
  });

  it("未进入模式时给出进入入口，且不渲染审批按钮", async () => {
    mockMode({ active: false, plan: null });

    const wrapper = mount(PlanPanel, { props: { sessionId: "session-1", open: true } });
    await flushAsync();

    expect(wrapper.get('[data-testid="plan-mode-card"]').text()).toContain("未激活");
    expect(wrapper.get('[data-testid="plan-mode-toggle"]').text()).toBe("进入");
    expect(wrapper.find('[data-testid="plan-review-actions"]').exists()).toBe(false);
    wrapper.unmount();
  });
});
