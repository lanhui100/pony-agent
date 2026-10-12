import { describe, it, expect, beforeEach } from "vitest";
import { ref } from "vue";
import { setActivePinia, createPinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { useProviderStore } from "@/stores/providers";
import { useTraceProjection } from "@/lib/runtime/useTraceProjection";
import type { TurnTraceRecord, TraceTimelineEntry } from "@/types/runtime";

describe("useTraceProjection - contextDisplayTokens computation", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("当单轮内发生多次工具调用（多跳）时，上下文窗口占用应展示最后一次实际调用的 inputTokens，而非整轮累加值", () => {
    const runtimeStore = useRuntimeStore();
    const providerStore = useProviderStore();

    // 模拟模型上下文上限为 1M (1,000,000)
    providerStore.$patch({
      selectedProviderId: "provider-gemini",
      providers: [
        {
          id: "provider-gemini",
          name: "Gemini",
          protocol: "gemini",
          baseUrl: "https://generativelanguage.googleapis.com",
          apiKeyEnvVar: "GEMINI_API_KEY",
          apiKeyValue: "",
          apiKeyPresent: true,
          selectedModelId: "gemini-1.5-pro",
          models: [
            {
              id: "gemini-1.5-pro",
              name: "Gemini 1.5 Pro",
              model: "gemini-1.5-pro",
              temperature: 0,
              maxOutputTokens: 8192,
              reasoningEffort: null,
              reasoningBudgetTokens: null,
              capabilityPreset: "gemini-pro",
              capabilities: {
                contextWindowTokens: 1_000_000,
                supportsTools: true,
                supportsStreaming: true,
                supportsImageInput: true,
                supportsReasoning: false
              }
            }
          ]
        }
      ]
    });

    // 模拟多跳 turn：
    // Hop 1: input 350K
    // Hop 2: input 360K
    // Hop 3: input 370K
    // 整轮累计 inputTokens = 1,080,000 (1.08M)，但物理上下文最后一次仅 370,000 (370K)
    const timeline: TraceTimelineEntry[] = [
      {
        id: "call-1",
        turnId: "turn-multihop",
        sequence: 1,
        kind: "call_model",
        label: "CALL MODEL #1",
        state: "completed",
        inputTokens: 350_000,
        outputTokens: 500
      },
      {
        id: "tool-1",
        turnId: "turn-multihop",
        sequence: 2,
        kind: "call_tool",
        label: "TOOL",
        state: "completed"
      },
      {
        id: "call-2",
        turnId: "turn-multihop",
        sequence: 3,
        kind: "call_model",
        label: "CALL MODEL #2",
        state: "completed",
        inputTokens: 360_000,
        outputTokens: 600
      },
      {
        id: "tool-2",
        turnId: "turn-multihop",
        sequence: 4,
        kind: "call_tool",
        label: "TOOL",
        state: "completed"
      },
      {
        id: "call-3",
        turnId: "turn-multihop",
        sequence: 5,
        kind: "call_model",
        label: "CALL MODEL #3",
        state: "completed",
        inputTokens: 370_000,
        outputTokens: 700
      }
    ];

    const turnRecord: TurnTraceRecord = {
      turnId: "turn-multihop",
      title: "多跳调用",
      phase: "completed",
      traceSteps: [],
      traceTimeline: timeline,
      toolActivities: [],
      // 整轮累计 inputTokens: 1.08M
      inputTokens: 1_080_000,
      outputTokens: 1800,
      updatedAt: Date.now()
    };

    runtimeStore.$patch({
      inputTokens: 1_080_000,
      turnTraceHistory: [turnRecord],
      traceTimeline: timeline,
      activeTurnId: null,
      isSubmitting: false
    });

    const projection = useTraceProjection({
      liveTurnEnabled: ref(true)
    });

    // 核心断言：
    // 1. sessionInputTokensTotal 依然是整轮累计的 1,080,000 (用户账单 Token)
    expect(projection.sessionInputTokensTotal.value).toBe(1_080_000);
    // 2. contextDisplayTokens 必须是最后一次模型调用的物理上下文 370,000，而不是累加的 1,080,000
    expect(projection.contextDisplayTokens.value).toBe(370_000);
  });

  it("当单轮只有单次调用且没有 call_model 条目指标时，平滑回退到 turn.inputTokens", () => {
    const runtimeStore = useRuntimeStore();
    const turnRecord: TurnTraceRecord = {
      turnId: "turn-single",
      title: "单次调用",
      phase: "completed",
      traceSteps: [],
      traceTimeline: [],
      toolActivities: [],
      inputTokens: 12_500,
      outputTokens: 400,
      updatedAt: Date.now()
    };

    runtimeStore.$patch({
      inputTokens: 12_500,
      turnTraceHistory: [turnRecord],
      activeTurnId: null,
      isSubmitting: false
    });

    const projection = useTraceProjection({
      liveTurnEnabled: ref(true)
    });

    expect(projection.contextDisplayTokens.value).toBe(12_500);
  });
});
