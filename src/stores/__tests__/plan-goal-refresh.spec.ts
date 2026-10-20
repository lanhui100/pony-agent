/**
 * Wave-1 冻结契约 C6 前端验收（Test Agent 冻结，实施者只读不可改）
 *
 * 契约唯一来源：`.dev-team/contract-wave-1.md` C6 / 验收 6 与
 * `docs/architecture/plan-goal-contract.md` §6（`refreshForTurn(sessionId)`）。
 *
 * 红相原则：空桩期（`throw new Error("NotImplemented: wave-1")`）必须**可运行且断言失败**；
 * 严禁 `it.skip` / 空断言 / 吞异常。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

// 空桩 `plan.ts` / `goal.ts` 均从 `@/lib/tauri` 引入 safeInvoke / isTauriAvailable，
// 故按现有 tests/*.spec.ts 的同类写法在该模块边界打桩。
vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

import { refreshForTurn as refreshPlanForTurn } from "@/stores/plan";
import { refreshForTurn as refreshGoalForTurn } from "@/stores/goal";
import { usePlanStore } from "@/stores/plan";
import { useGoalStore } from "@/stores/goal";

/** 从 mock 调用记录里取出「命令名 + 参数」二元组，便于稳定断言（不依赖调用顺序）。 */
function invokedCommands(): Array<{ command: string; args: Record<string, unknown> | undefined }> {
  return tauriMocks.mockSafeInvoke.mock.calls.map((call) => ({
    command: String(call[0]),
    args: call[1] as Record<string, unknown> | undefined
  }));
}

describe("wave-1 C6: turn 结束后的 plan / goal 刷新", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  it("plan.ts::refreshForTurn 以真实 sessionId 拉取 plan_list 与 todo_list", async () => {
    // Arrange
    tauriMocks.mockSafeInvoke.mockResolvedValue([]);
    const sessionId = "sess-1";

    // Act
    await refreshPlanForTurn(sessionId);

    // Assert：两条命令都必须以真实 sessionId 发起（禁止 "default" 兜底）
    const commands = invokedCommands();
    const planListCall = commands.find((call) => call.command === "plan_list");
    const todoListCall = commands.find((call) => call.command === "todo_list");

    expect(
      planListCall,
      `契约 C6 违约：refreshForTurn 必须以 sessionId 调用 plan_list；实际命令序列 = ${JSON.stringify(
        commands.map((call) => call.command)
      )}`
    ).toBeDefined();
    expect(planListCall?.args).toEqual({ sessionId });

    expect(
      todoListCall,
      `契约 C6 违约：refreshForTurn 必须以 sessionId 调用 todo_list；实际命令序列 = ${JSON.stringify(
        commands.map((call) => call.command)
      )}`
    ).toBeDefined();
    expect(todoListCall?.args).toEqual({ sessionId });

    // Assert：store 侧以真实 sessionId 落地（空计划也允许，但不得使用 "default"）
    const planStore = usePlanStore();
    expect(planStore.plans).toEqual([]);
    expect(planStore.todos).toEqual([]);
  });

  it("goal.ts::refreshForTurn 以真实 sessionId 拉取 goal_get", async () => {
    // Arrange
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    const sessionId = "sess-1";

    // Act
    await refreshGoalForTurn(sessionId);

    // Assert
    const commands = invokedCommands();
    const goalGetCall = commands.find((call) => call.command === "goal_get");

    expect(
      goalGetCall,
      `契约 C6 违约：refreshForTurn 必须以 sessionId 调用 goal_get；实际命令序列 = ${JSON.stringify(
        commands.map((call) => call.command)
      )}`
    ).toBeDefined();
    expect(goalGetCall?.args).toEqual({ sessionId });
    expect(useGoalStore().currentGoal).toBeNull();
  });

  it("空 sessionId 直接返回且不发起任何 ipc 调用，也不抛错", async () => {
    // Arrange
    tauriMocks.mockSafeInvoke.mockResolvedValue([]);

    // Act + Assert
    await expect(refreshPlanForTurn("")).resolves.toBeUndefined();
    await expect(refreshGoalForTurn("   ")).resolves.toBeUndefined();
    expect(invokedCommands()).toEqual([]);
  });

  it("invoke 抛错时 refreshForTurn 不得向上抛错（降级为空）", async () => {
    // Arrange
    tauriMocks.mockSafeInvoke.mockRejectedValue(new Error("ipc unavailable"));

    // Act + Assert：不得 rejects（禁止把 IPC 故障冒泡到 turn 结束路径）
    await expect(refreshPlanForTurn("sess-1")).resolves.toBeUndefined();
    await expect(refreshGoalForTurn("sess-1")).resolves.toBeUndefined();

    const planStore = usePlanStore();
    expect(planStore.plans).toEqual([]);
    expect(planStore.todos).toEqual([]);
    expect(useGoalStore().currentGoal).toBeNull();
  });
});
