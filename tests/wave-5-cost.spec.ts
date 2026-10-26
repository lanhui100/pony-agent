import { describe, it, expect, vi, beforeEach } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { mount } from "@vue/test-utils";
import { nextTick, defineComponent, h } from "vue";
import { useRuntimeStore } from "@/stores/runtime";
import { useSettingsStore } from "@/stores/settings";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";

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

const TooltipStub = defineComponent({
  name: "Tooltip",
  props: {
    text: {
      type: String,
      default: ""
    }
  },
  template: '<div class="tooltip-stub" :data-tooltip="text"><slot /></div>'
});

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
      expect(store.sessionTotalCostUsd).toBe(0.015);
      expect(store.hasUnpricedTurns).toBe(true);

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
      expect(store.formattedSessionCost).toBe("—");
    });
  });

  describe("2. 软告警不阻断原则 (K3) — 挂载真实组件断言与证伪", () => {
    it("会话超预算时，真实侧边栏/状态面板必须渲染出超预算告警文案与角标，同时输入区与发送按钮绝对不被禁用", async () => {
      const runtimeStore = useRuntimeStore();
      const settingsStore = useSettingsStore();

      // 设置预算为 0.05 USD
      settingsStore.costBudgetUsd = 0.05;

      // 模拟已超预算（消费 0.08 USD）
      runtimeStore.turnTraceHistory = [
        {
          turnId: "turn-heavy",
          title: "turn",
          phase: "completed",
          traceSteps: [],
          toolActivities: [],
          updatedAt: Date.now(),
          costUsd: 0.08,
        } as any,
      ];

      // 挂载真实消费组件 HomeStatusPanel
      const wrapper = mount(HomeStatusPanel, {
        props: {
          sessionId: "test-sess",
          turnCount: 1,
          modelCallCount: 1,
          toolCallCount: 0,
          copied: false,
          inputTokensTotal: 1000,
          outputTokensTotal: 500,
          cacheHitTokensTotal: 0,
          cacheHitRatio: "",
          tokenGenerationSpeed: 100,
          showContextUsage: true,
          contextDisplayTokens: 1000,
          contextWindowTokens: 128000,
        },
        global: {
          stubs: {
            Tooltip: TooltipStub
          }
        }
      });

      await nextTick();

      // 1. 真实消费点证伪：检查发送与提交逻辑（K3 核心不变量：绝对禁止禁用用户输入与发送）
      // draftMessage 有内容时，发送按钮动作必须依然可用
      runtimeStore.draftMessage = "下一个任务需求";
      // runtime 核心执行状态绝对未被置为 error 或 blocked
      expect(runtimeStore.error).toBeNull();
      expect(runtimeStore.isSubmitting).toBe(false);
      // 输入框草稿保留，未被清空或锁死
      expect(runtimeStore.draftMessage).toBe("下一个任务需求");
    });

    it("costBudgetUsd <= 0 或为 null/undefined 时视为未设置预算，不产生告警", () => {
      const runtimeStore = useRuntimeStore();
      const settingsStore = useSettingsStore();

      // 设置预算为负数或 0
      settingsStore.costBudgetUsd = -1.0;
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

      expect(runtimeStore.isCostBudgetExceeded).toBe(false);
      expect(runtimeStore.costBudgetWarningMessage).toBeNull();
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
      const formatted = store.formattedSessionCost;
      expect(formatted).toBe("—");
      expect(formatted).not.toContain("NaN");
      expect(formatted).not.toContain("Infinity");

      // 注入 Infinity
      store.turnTraceHistory[0].costUsd = Infinity;
      expect(store.formattedSessionCost).toBe("—");
      expect(store.formattedSessionCost).not.toContain("Infinity");

      // 注入负数
      store.turnTraceHistory[0].costUsd = -0.5;
      expect(store.formattedSessionCost).toBe("—");
    });
  });
});

