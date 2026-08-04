import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { usePlanStore } from "@/stores/plan";
import type { Plan, PlanStep } from "@/types/ask-plan";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

function createStep(partial: Partial<PlanStep> = {}): PlanStep {
  return {
    stepId: partial.stepId ?? "step-1",
    name: partial.name ?? "调研方案",
    summary: partial.summary ?? "梳理接入点",
    status: partial.status ?? "pending"
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
    steps: partial.steps ?? [createStep()]
  };
}

describe("plan store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  it("lists a session's plans through plan_list and auto-selects the first", async () => {
    const plans = [createPlan({ planId: "plan-1" }), createPlan({ planId: "plan-2" })];
    tauriMocks.mockSafeInvoke.mockResolvedValue(plans);

    const store = usePlanStore();
    const result = await store.list("session-1");

    expect(result).toHaveLength(2);
    expect(store.plans).toHaveLength(2);
    expect(store.selectedPlanId).toBe("plan-1");
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_list", {
      sessionId: "session-1"
    });
  });

  it("falls back to an empty list when the host returns null", async () => {
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);

    const store = usePlanStore();
    const result = await store.list("session-1");

    expect(result).toEqual([]);
    expect(store.plans).toEqual([]);
    expect(store.selectedPlan).toBeNull();
  });

  it("does not call the host in browser preview mode or without a session", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    const store = usePlanStore();
    expect(await store.list("")).toEqual([]);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalled();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    expect(await store.list("   ")).toEqual([]);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalled();
  });

  it("completes a step with the listed revision and stores the bumped-revision plan", async () => {
    const plan = createPlan({
      planId: "plan-1",
      revision: 2,
      lifecycle: "executing",
      steps: [createStep({ stepId: "step-1" }), createStep({ stepId: "step-2" })]
    });
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce([plan]).mockResolvedValueOnce(
      createPlan({
        planId: "plan-1",
        revision: 3,
        lifecycle: "executing",
        steps: [
          createStep({ stepId: "step-1", status: "completed" }),
          createStep({ stepId: "step-2" })
        ]
      })
    );

    const store = usePlanStore();
    await store.list("session-1");
    expect(store.selectedPlan?.revision).toBe(2);

    const updated = await store.completeStep("session-1", "plan-1", 2, "step-1");

    expect(updated?.revision).toBe(3);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_complete_step", {
      sessionId: "session-1",
      planId: "plan-1",
      revision: 2,
      stepId: "step-1"
    });
    expect(store.selectedPlan?.revision).toBe(3);
    expect(store.selectedPlan?.steps[0]?.status).toBe("completed");
    expect(store.completingStepKey).toBeNull();
  });

  it("refuses a second step completion while one is in flight", async () => {
    const plan = createPlan({ planId: "plan-1", revision: 1 });
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "plan_list") {
        return Promise.resolve([plan]);
      }
      if (command === "plan_complete_step") {
        return new Promise((resolve) => {
          setTimeout(() => resolve(plan), 10);
        });
      }
      return Promise.resolve(null);
    });

    const store = usePlanStore();
    await store.list("session-1");

    const first = store.completeStep("session-1", "plan-1", 1, "step-1");
    const second = store.completeStep("session-1", "plan-1", 1, "step-1");

    expect(store.completingStepKey).toBe("plan-1:step-1");
    expect(second).resolves.toBeNull();
    await first;
    expect(store.completingStepKey).toBeNull();
  });

  it("creates a draft plan through plan_create and selects it", async () => {
    const created = createPlan({
      planId: "plan-new",
      revision: 1,
      lifecycle: "draft",
      steps: []
    });
    tauriMocks.mockSafeInvoke.mockResolvedValue(created);

    const store = usePlanStore();
    const result = await store.create("session-1", {
      kind: "plan",
      summary: "新计划",
      steps: []
    });

    expect(result?.planId).toBe("plan-new");
    expect(store.selectedPlanId).toBe("plan-new");
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_create", {
      sessionId: "session-1",
      payload: { kind: "plan", summary: "新计划", steps: [] }
    });
  });

  it("reads a single plan through plan_get and upserts it", async () => {
    const plan = createPlan({ planId: "plan-9", revision: 4 });
    tauriMocks.mockSafeInvoke.mockResolvedValue(plan);

    const store = usePlanStore();
    const result = await store.get("session-1", "plan-9");

    expect(result?.planId).toBe("plan-9");
    expect(store.plans).toContainEqual(plan);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_get", {
      sessionId: "session-1",
      planId: "plan-9"
    });
  });

  it("appends a step through plan_merge using the listed revision", async () => {
    const updated = createPlan({
      planId: "plan-1",
      revision: 2,
      steps: [createStep(), createStep({ stepId: "step-2", name: "第二步" })]
    });
    tauriMocks.mockSafeInvoke.mockResolvedValue(updated);

    const store = usePlanStore();
    const result = await store.mergeStep("session-1", "plan-1", 1, {
      name: "第二步",
      summary: "追加的步骤"
    });

    expect(result?.revision).toBe(2);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_merge", {
      sessionId: "session-1",
      planId: "plan-1",
      revision: 1,
      step: { name: "第二步", summary: "追加的步骤" }
    });
  });

  it("replaces a plan through plan_replace preserving the plan id", async () => {
    const replaced = createPlan({ planId: "plan-1", revision: 5, summary: "替换后的计划" });
    tauriMocks.mockSafeInvoke.mockResolvedValue(replaced);

    const store = usePlanStore();
    const result = await store.replace("session-1", "plan-1", 4, {
      kind: "plan",
      summary: "替换后的计划"
    });

    expect(result?.summary).toBe("替换后的计划");
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("plan_replace", {
      sessionId: "session-1",
      planId: "plan-1",
      revision: 4,
      payload: { kind: "plan", summary: "替换后的计划" }
    });
  });

  it("keeps the selection valid after the selected plan disappears", async () => {
    tauriMocks.mockSafeInvoke.mockResolvedValue([
      createPlan({ planId: "plan-1" }),
      createPlan({ planId: "plan-2" })
    ]);

    const store = usePlanStore();
    await store.list("session-1");
    store.select("plan-2");
    expect(store.selectedPlanId).toBe("plan-2");

    tauriMocks.mockSafeInvoke.mockResolvedValue([createPlan({ planId: "plan-1" })]);
    await store.list("session-1");
    expect(store.selectedPlanId).toBe("plan-1");
  });
});
