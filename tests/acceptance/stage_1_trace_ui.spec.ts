import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import TrajectoryTimeline from "@/components/trajectory/TrajectoryTimeline.vue";
import HomeTracePanel from "@/components/HomeTracePanel.vue";
import type { TrajectoryTimelineModel } from "@/lib/runtime/trajectory-core";
import type { TurnTraceRecord } from "@/types/runtime";

describe("Stage 1 Trace UI Acceptance Tests (Border Removal & Structured Overview Metrics)", () => {
  describe("TrajectoryTimeline: border removal and subtle styling", () => {
    const mockModel: TrajectoryTimelineModel = {
      totalStart: 0,
      totalEnd: 1000,
      totalDurationMs: 1000,
      turnBoundaries: [
        { turnId: "turn-1", title: "Turn 1", time: 0 },
      ],
      spans: [
        {
          id: "span-user",
          turnId: "turn-1",
          index: 0,
          kind: "user",
          label: "User Input",
          lane: 0,
          start: 0,
          end: 100,
          durationMs: 100,
          isError: false,
        },
        {
          id: "span-model",
          turnId: "turn-1",
          index: 1,
          kind: "message",
          label: "DeepSeek Chat",
          lane: 1,
          start: 100,
          end: 600,
          durationMs: 500,
          isError: false,
        },
        {
          id: "span-tool",
          turnId: "turn-1",
          index: 2,
          kind: "tool",
          label: "read_file",
          lane: 2,
          start: 600,
          end: 800,
          durationMs: 200,
          isError: false,
        },
        {
          id: "span-error",
          turnId: "turn-1",
          index: 3,
          kind: "message",
          label: "Error Step",
          lane: 1,
          start: 800,
          end: 1000,
          durationMs: 200,
          isError: true,
        },
      ],
    };

    it("verifies that span buttons do NOT contain any 'border' or 'border-*' classes", () => {
      const wrapper = mount(TrajectoryTimeline, {
        props: {
          model: mockModel,
          selectedSpanId: null,
        },
        global: {
          stubs: {
            Tooltip: {
              template: `<div><slot /></div>`,
            },
          },
        },
      });

      const buttons = wrapper.findAll("button[data-testid^='trajectory-span-']");
      expect(buttons.length).toBeGreaterThan(0);

      for (const btn of buttons) {
        const classList = btn.classes();
        const borderClasses = classList.filter((cls) => cls === "border" || cls.startsWith("border-"));
        expect(
          borderClasses,
          `Span button ${btn.attributes("data-testid")} should not contain border classes, but got: ${borderClasses.join(", ")}`
        ).toEqual([]);
      }
    });
  });

  describe("HomeTracePanel: structured dl/dt/dd metrics overview", () => {
    const mockTurnWithMetrics: TurnTraceRecord = {
      turnId: "turn-metric-1",
      title: "Model Turn",
      phase: "completed",
      updatedAt: 1000,
      turnDurationMs: 1200,
      firstTokenLatencyMs: 150,
      traceSteps: [],
      toolActivities: [],
      traceTimeline: [
        {
          id: "step-model-1",
          kind: "call_model",
          label: "DeepSeek Chat",
          state: "completed",
          sequence: 1,
          turnDurationMs: 1200,
          durationMs: 1200,
          firstTokenLatencyMs: 150,
          inputTokens: 1200,
          outputTokens: 400,
          reasoningTokens: 100,
          cacheHitInputTokens: 300,
          providerName: "DeepSeek",
          providerModel: "deepseek-chat",
        },
      ],
    };

    it("verifies that call_model step details render a structured dl/dt/dd or overview metric panel containing core metrics", async () => {
      const wrapper = mount(HomeTracePanel, {
        props: {
          turns: [mockTurnWithMetrics],
          sessionId: "test-session",
          copiedKey: "",
          open: true,
          canonicalKind: (kind) => kind,
          turnTimeline: (turn) => turn.traceTimeline || [],
          providerReturnedCacheHitInputTokens: () => 300,
          expanded: true,
        },
        global: {
          stubs: {
            Tooltip: {
              template: `<div><slot /></div>`,
            },
            ScrollArea: {
              template: `<div><slot /></div>`,
            },
          },
        },
      });

      // Expand the turn first
      const turnButton = wrapper.find("button.group");
      expect(turnButton.exists()).toBe(true);
      await turnButton.trigger("click");

      // Expand the call_model step
      const stepButton = wrapper.find("[data-testid='trace-step-button-step-model-1']");
      expect(stepButton.exists()).toBe(true);
      await stepButton.trigger("click");

      // Check for structured dl/dt/dd metric panel (dl with class containing overview or metric grid)
      const dlElements = wrapper.findAll("dl");
      expect(
        dlElements.length,
        "Expected at least one <dl> definition list for structured metrics overview"
      ).toBeGreaterThan(0);

      const metricPanelText = dlElements.map((el) => el.text()).join(" ");

      // Verify core metrics are present in the structured description list:
      // 1. 输入 (Input Tokens)
      // 2. 输出 (Output Tokens)
      // 3. 缓存 (Cache Read / Hit)
      // 4. 首 Token 延迟 / TTFT
      // 5. 吞吐 / 生成速度 (Throughput / Speed)
      expect(metricPanelText).toMatch(/输入|Input/i);
      expect(metricPanelText).toMatch(/输出|Output/i);
      expect(metricPanelText).toMatch(/缓存|Cache/i);
      expect(metricPanelText).toMatch(/首\s*token|ttft|首包延时|首次延迟/i);
      expect(metricPanelText).toMatch(/速度|吞吐|Throughput/i);
    });
  });
});
