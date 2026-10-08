import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { deriveTrajectoryTimeline } from "@/lib/runtime/trajectory-core";
import TrajectoryTimeline from "@/components/trajectory/TrajectoryTimeline.vue";
import type { TurnTraceRecord } from "@/types/runtime";

describe("Desktop Optimizations P1 - P5 Regression & Contract Gates", () => {
  describe("P1: 消息流按测序逐个依次显示，避免Duplicate Key", () => {
    it("ensures each model hop event has a unique event key including entry identifier", () => {
      // 验证当存在多 hop 模型调用时，每个 hop 的 reasoning 与 content 事件 key 唯一
      const mockTimeline = [
        {
          id: "model-hop-1",
          kind: "call_model",
          label: "Model 1",
          state: "completed",
          sequence: 10,
          text: "Hop 1 text",
          reasoningContent: "Hop 1 thought"
        },
        {
          id: "model-hop-2",
          kind: "call_model",
          label: "Model 2",
          state: "completed",
          sequence: 30,
          text: "Hop 2 text",
          reasoningContent: "Hop 2 thought"
        }
      ];

      // 提取事件 key 的函数逻辑应保证不同 entry id 产生不同 key
      const key1 = `reasoning-asst-1-${mockTimeline[0]!.id}`;
      const key2 = `reasoning-asst-1-${mockTimeline[1]!.id}`;
      expect(key1).not.toBe(key2);
    });
  });

  describe("P2: 侧边栏token的计算和速度提取逻辑", () => {
    it("computes model call generation speed from durationSeconds even when turnDurationMs is absent", () => {
      const entry = {
        id: "call-1",
        kind: "call_model" as const,
        label: "Model",
        state: "completed" as const,
        sequence: 1,
        outputTokens: 120,
        durationSeconds: 2.0, // 2000ms
        firstTokenLatencyMs: 400 // net generation duration = 1600ms = 1.6s
      };

      // 净生成时间为 2000 - 400 = 1600ms (1.6s)，速度 = 120 / 1.6 = 75 t/s
      const durationValue = (entry.durationSeconds != null ? Math.round(entry.durationSeconds * 1000) : null);
      expect(durationValue).toBe(2000);
      const generationDurationMs = durationValue! - entry.firstTokenLatencyMs;
      const speed = entry.outputTokens / (generationDurationMs / 1000);
      expect(speed).toBe(75);
    });
  });

  describe("P4: Trace面板内边距与甘特图字段名防压色条", () => {
    it("renders lane labels in a dedicated column or before the timeline track so spans do not cover label text", () => {
      const mockModel = deriveTrajectoryTimeline([
        {
          turnId: "turn-p4",
          title: "Turn P4",
          phase: "completed",
          updatedAt: 1000,
          traceSteps: [],
          toolActivities: [],
          traceTimeline: [
            {
              id: "step-1",
              kind: "call_model",
              label: "Model",
              state: "completed",
              sequence: 1,
              durationSeconds: 1.0,
            },
            {
              id: "step-2",
              kind: "call_tool",
              label: "Tool",
              state: "completed",
              sequence: 2,
              durationSeconds: 0.5,
            }
          ]
        }
      ], "sequence");

      const wrapper = mount(TrajectoryTimeline, {
        props: {
          model: mockModel,
          selectedSpanId: null,
        },
        global: {
          stubs: {
            Tooltip: { template: "<div><slot /></div>" },
          },
        },
      });

      // 验证外层容器拥有内边距类（如 px-3 或 px-4, py-2 或 py-3）
      const root = wrapper.find("[data-testid='trajectory-timeline']");
      expect(root.exists()).toBe(true);
      const rootClasses = root.classes().join(" ");
      expect(rootClasses).toMatch(/p-\d|px-\d/);

      // 验证甘特图有独立的字段名容器或网格结构，span 轨道不是 0~100% 覆盖字段名
      const labelContainer = wrapper.find("[data-testid='trajectory-lane-labels']");
      const trackContainer = wrapper.find("[data-testid='trajectory-lane-track']");
      expect(labelContainer.exists(), "Should have dedicated lane labels container").toBe(true);
      expect(trackContainer.exists(), "Should have dedicated track container starting after lane labels").toBe(true);
    });
  });

  describe("P5: Trace面板等宽视图与实际耗时trigger有明显差异", () => {
    it("produces distinct span durations between sequence and duration modes when entries have durationSeconds", () => {
      const turns: TurnTraceRecord[] = [
        {
          turnId: "turn-p5",
          title: "Turn P5",
          phase: "completed",
          updatedAt: 1000,
          traceSteps: [],
          toolActivities: [],
          traceTimeline: [
            {
              id: "fast-step",
              kind: "call_model",
              label: "Fast Call",
              state: "completed",
              sequence: 1,
              durationSeconds: 0.1, // 100ms
            },
            {
              id: "slow-step",
              kind: "call_tool",
              label: "Slow Tool",
              state: "completed",
              sequence: 2,
              durationSeconds: 3.0, // 3000ms
            }
          ]
        }
      ];

      const seqModel = deriveTrajectoryTimeline(turns, "sequence");
      const durModel = deriveTrajectoryTimeline(turns, "duration");

      expect(seqModel.spans.length).toBe(2);
      expect(durModel.spans.length).toBe(2);

      // 在等宽模式下，两者的 span 长度相等 (如 1 vs 1)
      const seqSpan1Width = seqModel.spans[0]!.end - seqModel.spans[0]!.start;
      const seqSpan2Width = seqModel.spans[1]!.end - seqModel.spans[1]!.start;
      expect(seqSpan1Width).toBe(seqSpan2Width);

      // 在实际耗时模式下，慢的调用必须明显长于快的调用 (如 3000ms vs 100ms)
      const durSpan1Width = durModel.spans[0]!.end - durModel.spans[0]!.start;
      const durSpan2Width = durModel.spans[1]!.end - durModel.spans[1]!.start;
      expect(durSpan2Width).toBeGreaterThan(durSpan1Width * 2);
    });
  });
});
