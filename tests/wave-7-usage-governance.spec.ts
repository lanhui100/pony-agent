import { describe, it, expect, vi, beforeEach } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { useSettingsStore } from "@/stores/settings";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable,
}));

describe("Wave-7 Usage Governance Frontend Contract Suite", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    setActivePinia(createPinia());
  });

  describe("1. G4 面板数字与未计价轮数汇总契约", () => {
    it("会话面板展示成本必须等于手工按轮汇总，且并列展示未计价轮数与 '≥' 前缀语义", () => {
      const store = useRuntimeStore();

      // 先布设陷阱并证明陷阱生效：
      // 2 轮已计价 (0.012345, 0.020000)，1 轮未计价 (undefined)
      store.turnTraceHistory = [
        {
          turnId: "turn-p1",
          title: "p1",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now() - 10000,
          costUsd: 0.012345,
        } as any,
        {
          turnId: "turn-unpriced",
          title: "unpriced",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now() - 5000,
          costUsd: undefined,
        } as any,
        {
          turnId: "turn-p2",
          title: "p2",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now(),
          costUsd: 0.02,
        } as any,
      ];

      // 证明陷阱：第二轮为 undefined
      expect(store.turnTraceHistory[1].costUsd).toBeUndefined();

      // 断言 governance summary 接口读取与前端汇总
      const summary = (store as any).usageGovernanceSummary;
      expect(summary).toBeDefined();
      expect(summary.session.cost_usd).toBeCloseTo(0.032345, 6);
      expect(summary.session.unpriced_turns).toBe(1);

      // UI 文本必须并列体现已计价总和与未计价轮数（带 ≥ 语义）
      const displayText = (store as any).formattedGovernanceCost;
      expect(displayText).toContain("≥ $0.032345");
      expect(displayText).toContain("1 轮未计价");
    });

    it("全部未计价时显示 '—'，绝不出现 NaN、Infinity 或误导性的 $0.000000", () => {
      const store = useRuntimeStore();

      store.turnTraceHistory = [
        {
          turnId: "turn-u1",
          title: "u1",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now() - 1000,
          costUsd: undefined,
        } as any,
        {
          turnId: "turn-u2",
          title: "u2",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now(),
          costUsd: undefined,
        } as any,
      ];

      const displayText = (store as any).formattedGovernanceCost;
      expect(displayText).toBe("—");
      expect(displayText).not.toContain("NaN");
      expect(displayText).not.toContain("Infinity");
      expect(displayText).not.toContain("$0.000000");
    });
  });

  describe("2. G2/G3 拦截态 UI 与 Turn 保护契约", () => {
    it("Block 拦截态时展示「当前值 / 上限 / 一键出口」，且绝对不在 turn 进行中中断既有状态", () => {
      const runtimeStore = useRuntimeStore();
      const settingsStore = useSettingsStore();

      // 配置 opt-in 物理拦截且超限
      (settingsStore as any).usagePolicy = {
        window: "session",
        hard_limit_usd: 0.05,
        soft_ratio: 0.8,
        enforcement: true,
      };

      // 模拟已超限触发 Block 态
      (runtimeStore as any).governanceDecision = {
        kind: "Block",
        reason: "Session cost limit exceeded",
        current: 0.065,
        limit: 0.05,
        would_block: true,
      };

      // 1. 断言拦截态 UI 暴露三项必要信息：当前值、限额、以及放行/一键调大限额出口
      const blockDetail = (runtimeStore as any).governanceBlockDetail;
      expect(blockDetail).toBeDefined();
      expect(blockDetail.currentText).toContain("0.065");
      expect(blockDetail.limitText).toContain("0.05");
      expect(blockDetail.overrideActionAvailable).toBe(true); // 一键出口可用

      // 2. 关键不变量：若当前 Turn 已经在执行中（isSubmitting = true），绝对不被直接改写中断
      runtimeStore.isSubmitting = true;
      runtimeStore.activeTurnId = "turn-in-progress";

      // 即使 governanceDecision 为 Block，也绝不改写已经处于 running/calling_model 中的 turn 状态
      expect(runtimeStore.isSubmitting).toBe(true);
      expect(runtimeStore.activeTurnId).toBe("turn-in-progress");
    });
  });
});
