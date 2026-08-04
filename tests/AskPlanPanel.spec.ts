import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import AskPanel from "@/components/AskPanel.vue";
import PlanPanel from "@/components/PlanPanel.vue";
import { useAskStore } from "@/stores/ask";
import { usePlanStore } from "@/stores/plan";
import type { PendingAsk, Plan } from "@/types/ask-plan";

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

function createPlan(partial: Partial<Plan> = {}): Plan {
  return {
    planId: partial.planId ?? "plan-1",
    sessionId: partial.sessionId ?? "session-1",
    revision: partial.revision ?? 1,
    kind: partial.kind ?? "plan",
    summary: partial.summary ?? "接入 Ask/Plan 面板",
    lifecycle: partial.lifecycle ?? "draft",
    steps: partial.steps ?? [
      { stepId: "step-1", name: "调研方案", summary: "梳理接入点", status: "pending" }
    ]
  };
}

async function flushAsync() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

describe("AskPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders nothing when there are no pending asks", async () => {
    tauriMocks.mockSafeInvoke.mockResolvedValue([]);

    const wrapper = mount(AskPanel);
    await flushAsync();

    expect(wrapper.find('[data-testid="ask-panel"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("renders a pending ask prompt, options, and answer controls", async () => {
    const ask = createPendingAsk({
      requestId: "ask-1",
      prompt: "确认继续运行？",
      options: ["继续", "暂停"]
    });
    tauriMocks.mockSafeInvoke.mockResolvedValue([ask]);

    const wrapper = mount(AskPanel);
    await flushAsync();
    await vi.waitFor(() => {
      expect(useAskStore().pendingAsks).toHaveLength(1);
    });

    expect(wrapper.get('[data-testid="ask-panel"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="ask-prompt"]').text()).toContain("确认继续运行？");
    expect(wrapper.get('[data-testid="ask-options"]').text()).toContain("继续");
    expect(wrapper.get('[data-testid="ask-options"]').text()).toContain("暂停");
    expect(wrapper.get('[data-testid="ask-answer-submit"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="ask-cancel-submit"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("answers through the store using the listed version and removes the card", async () => {
    const ask = createPendingAsk({ requestId: "ask-1", version: 4, options: ["继续"] });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_answer") {
        return Promise.resolve({ ok: true });
      }
      return Promise.resolve(null);
    });

    const wrapper = mount(AskPanel);
    await flushAsync();
    const store = useAskStore();
    await vi.waitFor(() => {
      expect(store.pendingAsks).toHaveLength(1);
    });

    await wrapper.get('[data-testid="ask-options"] button').trigger("click");
    await flushAsync();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("ask_answer", {
      requestId: "ask-1",
      expectedVersion: 4,
      answer: "继续"
    });
    expect(store.pendingAsks).toHaveLength(0);
    wrapper.unmount();
  });

  it("disables the answer and cancel buttons while a request is being answered", async () => {
    const ask = createPendingAsk({ requestId: "ask-1" });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      if (command === "ask_answer") {
        return new Promise((resolve) => {
          setTimeout(() => resolve({ ok: true }), 15);
        });
      }
      return Promise.resolve(null);
    });

    const wrapper = mount(AskPanel);
    await flushAsync();

    const store = useAskStore();
    await vi.waitFor(() => {
      expect(store.pendingAsks).toHaveLength(1);
    });
    store.answeringRequestId = "ask-1";
    await wrapper.vm.$nextTick();

    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="ask-cancel-submit"]').attributes("disabled")).toBeDefined();
    wrapper.unmount();
  });
});

describe("PlanPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  it("lists a session's plans and shows the selected plan's steps", async () => {
    const plan = createPlan({
      planId: "plan-1",
      summary: "接入面板",
      lifecycle: "executing",
      steps: [
        { stepId: "step-1", name: "调研", summary: "梳理接入点", status: "pending" },
        { stepId: "step-2", name: "实现", summary: "编写代码", status: "completed" }
      ]
    });
    tauriMocks.mockSafeInvoke.mockResolvedValue([plan]);

    const wrapper = mount(PlanPanel, { props: { sessionId: "session-1", open: true } });
    await flushAsync();
    await vi.waitFor(() => {
      expect(usePlanStore().plans).toHaveLength(1);
    });

    expect(wrapper.text()).toContain("接入面板");
    expect(wrapper.text()).toContain("执行中");
    expect(wrapper.get('[data-testid="plan-steps"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="plan-step-step-1"]').text()).toContain("调研");
    expect(wrapper.get('[data-testid="plan-step-step-2"]').text()).toContain("实现");
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_list", {
      sessionId: "session-1"
    });
    wrapper.unmount();
  });

  it("completes a pending step through the store using the listed revision", async () => {
    const plan = createPlan({
      planId: "plan-1",
      revision: 3,
      lifecycle: "executing",
      steps: [{ stepId: "step-1", name: "调研", summary: "梳理接入点", status: "pending" }]
    });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "plan_list") {
        return Promise.resolve([plan]);
      }
      if (command === "plan_complete_step") {
        return Promise.resolve(
          createPlan({
            planId: "plan-1",
            revision: 4,
            lifecycle: "executing",
            steps: [{ stepId: "step-1", name: "调研", summary: "梳理接入点", status: "completed" }]
          })
        );
      }
      return Promise.resolve(null);
    });

    const wrapper = mount(PlanPanel, { props: { sessionId: "session-1", open: true } });
    await flushAsync();
    const store = usePlanStore();
    await vi.waitFor(() => {
      expect(store.selectedPlan?.revision).toBe(3);
    });

    await wrapper.get('[data-testid="plan-step-complete"]').trigger("click");
    await flushAsync();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_complete_step", {
      sessionId: "session-1",
      planId: "plan-1",
      revision: 3,
      stepId: "step-1"
    });
    expect(store.selectedPlan?.revision).toBe(4);
    expect(store.selectedPlan?.steps[0]?.status).toBe("completed");
    wrapper.unmount();
  });
});
