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

describe("Wave-5 Session Cost Estimation & Soft Warning Suite", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    setActivePinia(createPinia());
  });

  describe("1. 会话成本聚合与未知模型处理 (K2)", () => {
    it("turn 带 costUsd 时计算累计值；某轮缺失（未知模型）显示「—」且不按 0 计入", () => {
      const store = useRuntimeStore();

      // 先布设陷阱并证明陷阱已生效：
      // turn 1：计价成功，costUsd = 0.015000
      // turn 2：未知模型，costUsd 缺失为 undefined
      store.turnTraceHistory = [
        {
          turnId: "turn-known-1",
          title: "Known model turn",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now() - 5000,
          costUsd: 0.015,
        } as any,
        {
          turnId: "turn-unknown-2",
          title: "Unknown model turn",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now() - 1000,
          costUsd: undefined, // 未知模型计费缺失
        } as any,
      ];

      // 证明陷阱生效：第二轮确为 undefined
      expect(store.turnTraceHistory[1].costUsd).toBeUndefined();

      // 断言 store 上的聚合 getter：
      // sessionTotalCostUsd 聚合已知部分（0.015），且提供 hasUnpricedTurns 标记
      expect((store as any).sessionTotalCostUsd).toBe(0.015);
      expect((store as any).hasUnpricedTurns).toBe(true);

      // 若全会话无任何有效计价，显示占位符「—」
      store.turnTraceHistory = [
        {
          turnId: "turn-unpriced-only",
          title: "Unpriced turn",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now(),
          costUsd: undefined,
        } as any,
      ];
      expect((store as any).formattedSessionCost).toBe("—");
    });
  });

  describe("2. 软告警不阻断原则 (K3)", () => {
    it("会话超预算时出现告警文案/状态，但发送按钮与提交逻辑绝对不被阻断 (disabled === false)", () => {
      const runtimeStore = useRuntimeStore();
      const settingsStore = useSettingsStore();

      // 设置预算为 0.05 USD
      (settingsStore as any).costBudgetUsd = 0.05;

      // 会话累计消费 0.08 USD（超预算）
      runtimeStore.turnTraceHistory = [
        {
          turnId: "turn-overbudget",
          title: "Heavy turn",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now(),
          costUsd: 0.08,
        } as any,
      ];

      // 断言超预算告警状态已激活
      expect((runtimeStore as any).isCostBudgetExceeded).toBe(true);
      expect((runtimeStore as any).costBudgetWarningMessage).toContain("超预算");

      // 核心不变量：绝对禁止禁用发送或阻止用户输入
      expect((runtimeStore as any).canSubmitPrompt).toBe(true);
      expect((runtimeStore as any).isInputDisabled).toBe(false);
    });

    it("costBudgetUsd <= 0 或为 null/undefined 时视为未设置预算，不产生告警", () => {
      const runtimeStore = useRuntimeStore();
      const settingsStore = useSettingsStore();

      // 设置预算为负数或 0
      (settingsStore as any).costBudgetUsd = -1.0;
      runtimeStore.turnTraceHistory = [
        {
          turnId: "turn-normal",
          title: "turn",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now(),
          costUsd: 0.1,
        } as any,
      ];

      expect((runtimeStore as any).isCostBudgetExceeded).toBe(false);
      expect((runtimeStore as any).costBudgetWarningMessage).toBeNull();
    });
  });

  describe("3. 异常边界与对抗防御 (L2-AT)", () => {
    it("costUsd 为负 / NaN / Infinity 时安全退化为「—」，严禁向 UI 渲染出 NaN/Infinity 文本", () => {
      const store = useRuntimeStore();

      // 注入畸形数值
      store.turnTraceHistory = [
        {
          turnId: "turn-nan",
          title: "Malicious turn",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now(),
          costUsd: NaN,
        } as any,
      ];

      // 证明陷阱生效：
      expect(Number.isNaN(store.turnTraceHistory[0].costUsd)).toBe(true);

      // 断言格式化展示安全退化为 "—"，且不等于 "NaN" 或 "Infinity"
      const formatted = (store as any).formattedSessionCost;
      expect(formatted).toBe("—");
      expect(formatted).not.toContain("NaN");
      expect(formatted).not.toContain("Infinity");

      // 注入 Infinity
      store.turnTraceHistory[0].costUsd = Infinity;
      expect((store as any).formattedSessionCost).toBe("—");
      expect((store as any).formattedSessionCost).not.toContain("Infinity");

      // 注入负数
      store.turnTraceHistory[0].costUsd = -0.5;
      expect((store as any).formattedSessionCost).toBe("—");
    });
  });
});
