import { describe, expect, it, beforeEach } from "vitest";
import {
  clearTraceProjectionMemo,
  computeTurnTimeline,
  turnTimeline
} from "@/lib/runtime/trace-projection";
import type { TurnTraceRecord } from "@/types/runtime";

function createTraceRecord(overrides: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: overrides.turnId ?? "turn-1",
    title: overrides.title ?? "测试轮次",
    phase: overrides.phase ?? "ready",
    traceSteps: [],
    traceTimeline: overrides.traceTimeline ?? [],
    toolActivities: [],
    providerRequestedName: null,
    providerName: null,
    providerProtocol: null,
    providerModel: null,
    providerSource: null,
    providerMode: null,
    buildContextObservation: null,
    sessionSummary: null,
    fallbackReason: null,
    error: null,
    inputTokens: null,
    cacheHitInputTokens: null,
    reasoningTokens: null,
    outputTokens: null,
    totalTokens: null,
    firstTokenLatencyMs: null,
    turnDurationMs: null,
    updatedAt: overrides.updatedAt ?? 1,
    providerCallRecords: []
  };
}

describe("trace-projection", () => {
  beforeEach(() => {
    clearTraceProjectionMemo();
  });

  it("归一化：剔除 prepare_retrieval，return_result 折叠进 model 条目", () => {
    const turn = createTraceRecord({
      traceTimeline: [
        { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1 },
        { id: "prep-2", kind: "prepare_retrieval", label: "PREPARE RETRIEVAL", state: "completed", sequence: 2 },
        { id: "model-3", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 3 },
        { id: "return-4", kind: "return", label: "RETURN RESULT", state: "completed", sequence: 4, inputTokens: 120 }
      ]
    });

    const result = computeTurnTimeline(turn);
    expect(result.map((entry) => entry.kind)).toEqual(["input", "call_model"]);
    expect(result.some((entry) => entry.label === "PREPARE RETRIEVAL")).toBe(false);
    // return_result 折叠进 model：inputTokens 合并
    expect(result[1]).toMatchObject({ kind: "call_model", inputTokens: 120 });
  });

  it("签名化 memo：引用与 updatedAt 未变时命中缓存（引用相等）", () => {
    const timeline = [
      { id: "model-1", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 1 }
    ];
    const turn = createTraceRecord({ traceTimeline: timeline, updatedAt: 100 });

    const first = turnTimeline(turn);
    const second = turnTimeline(turn);
    expect(second).toBe(first);
  });

  it("签名化 memo：updatedAt 变化时失效重算", () => {
    const timeline = [
      { id: "model-1", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 1 }
    ];
    const turn = createTraceRecord({ traceTimeline: timeline, updatedAt: 100 });

    const first = turnTimeline(turn);
    turn.updatedAt = 200;
    const second = turnTimeline(turn);
    expect(second).not.toBe(first);
  });

  it("签名化 memo：timeline 引用变化时失效重算", () => {
    const turn = createTraceRecord({
      traceTimeline: [
        { id: "model-1", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 1 }
      ],
      updatedAt: 100
    });

    const first = turnTimeline(turn);
    turn.traceTimeline = [
      { id: "model-1", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 1 },
      { id: "tool-2", kind: "tool", label: "CALL TOOL", state: "completed", sequence: 2 }
    ];
    const second = turnTimeline(turn);
    expect(second).not.toBe(first);
    expect(second).toHaveLength(2);
  });

  it("clearTraceProjectionMemo 清空缓存", () => {
    const turn = createTraceRecord({
      traceTimeline: [
        { id: "model-1", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 1 }
      ]
    });

    const first = turnTimeline(turn);
    clearTraceProjectionMemo();
    const second = turnTimeline(turn);
    expect(second).not.toBe(first);
  });

  it("空 timeline 返回空数组", () => {
    const turn = createTraceRecord({ traceTimeline: [] });
    expect(computeTurnTimeline(turn)).toEqual([]);
  });
});