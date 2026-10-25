import { describe, it, expect } from "vitest";
import {
  resolveCallGenerationSpeed,
  resolveTurnGenerationSpeed,
  resolveContextUsageMetrics,
  type ContextUsageDebugInfo,
} from "@/lib/runtime/token-metrics";
import type { ProviderCallCacheRecord, TurnTraceRecord } from "@/types/runtime";

describe("Token Speed & Context Usage Metrics Suite (task-7)", () => {
  describe("1. S3 速度双模计算契约 (L2-T)", () => {
    it("模式 A：生成中/活跃调用存在时，优先按最近一次调用计算生成速度 (output / (duration - TTFT))", () => {
      const record: ProviderCallCacheRecord = {
        requestKind: "initial_request",
        providerSource: "primary",
        providerMode: "standard",
        latencyKind: "provider_stream",
        inputTokens: 1000,
        outputTokens: 100,
        turnDurationMs: 1200,
        firstTokenLatencyMs: 200, // generationMs = 1200 - 200 = 1000ms = 1s
      };

      const speed = resolveCallGenerationSpeed(record);
      // 100 tokens / 1.0s = 100 tokens/s
      expect(speed).toBeCloseTo(100, 2);
    });

    it("模式 B：轮次完成（isSubmitting=false 或处于终态）时，速度计算为整轮综合速度 (turn.outputTokens / turn.turnDurationMs)", () => {
      const completedTurn: TurnTraceRecord = {
        turnId: "turn-completed-1",
        status: "completed",
        inputTokens: 5000,
        outputTokens: 250,
        turnDurationMs: 2500, // 2.5s
        providerCallRecords: [
          {
            requestKind: "initial_request",
            providerSource: "primary",
            providerMode: "standard",
            latencyKind: "provider_stream",
            inputTokens: 5000,
            outputTokens: 250,
            turnDurationMs: 2500,
            firstTokenLatencyMs: 500,
          },
        ],
      };

      // 处于终态 (isSubmitting = false)
      const speed = resolveTurnGenerationSpeed(completedTurn, { isSubmitting: false });
      // 250 tokens / 2.5s = 100 tokens/s (综合整轮速度)
      expect(speed).toBeCloseTo(100, 2);
    });

    it("生成中（isSubmitting = true）时，优先采用最近活跃调用并抵扣有效 TTFT", () => {
      const activeTurn: TurnTraceRecord = {
        turnId: "turn-active-1",
        status: "running",
        inputTokens: 3000,
        outputTokens: 150,
        turnDurationMs: 3000,
        providerCallRecords: [
          {
            requestKind: "initial_request",
            providerSource: "primary",
            providerMode: "standard",
            latencyKind: "provider_stream",
            inputTokens: 3000,
            outputTokens: 150,
            turnDurationMs: 2000,
            firstTokenLatencyMs: 500, // 2000 - 500 = 1500ms = 1.5s -> 150 / 1.5 = 100 t/s
          },
        ],
      };

      const speed = resolveTurnGenerationSpeed(activeTurn, { isSubmitting: true });
      expect(speed).toBeCloseTo(100, 2);
    });
  });

  describe("2. 异常边界与红军对抗 (L2-AT)", () => {
    it("防御对抗：durationMs <= 0 时优雅返回 null，不产生除以零或 Infinity", () => {
      const zeroDurationRecord: ProviderCallCacheRecord = {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        outputTokens: 50,
        turnDurationMs: 0,
      };
      expect(resolveCallGenerationSpeed(zeroDurationRecord)).toBeNull();

      const negativeDurationRecord: ProviderCallCacheRecord = {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        outputTokens: 50,
        turnDurationMs: -100,
      };
      expect(resolveCallGenerationSpeed(negativeDurationRecord)).toBeNull();
    });

    it("防御对抗：TTFT >= durationMs 视为不可信，退化为不抵扣 TTFT", () => {
      const untrustedTtftRecord: ProviderCallCacheRecord = {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        outputTokens: 100,
        turnDurationMs: 1000,
        firstTokenLatencyMs: 1500, // TTFT > duration
      };
      // 不抵扣，按 100 / (1000 / 1000) = 100 t/s
      const speed = resolveCallGenerationSpeed(untrustedTtftRecord);
      expect(speed).toBeCloseTo(100, 2);
    });

    it("防御对抗：outputTokens 为空、null 或 undefined 时返回 null", () => {
      const noOutputRecord: ProviderCallCacheRecord = {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        outputTokens: undefined,
        turnDurationMs: 1000,
      };
      expect(resolveCallGenerationSpeed(noOutputRecord)).toBeNull();
    });

    it("多调用 fallback：当 records 为空或均无效时，单调用允许退化回 turn 级指标", () => {
      const singleCallTurn: TurnTraceRecord = {
        turnId: "turn-single",
        status: "completed",
        outputTokens: 120,
        turnDurationMs: 2000,
        providerCallRecords: [],
      };

      const speed = resolveTurnGenerationSpeed(singleCallTurn, { isSubmitting: false });
      // 120 / 2.0 = 60 t/s
      expect(speed).toBeCloseTo(60, 2);
    });
  });

  describe("3. 上下文用量与诊断信息透出契约 (L2-T)", () => {
    it("正确计算 contextUsagePercentage 与输出完整的 contextUsageDebugInfo", () => {
      const turn: TurnTraceRecord = {
        turnId: "turn-diag-1",
        status: "completed",
        inputTokens: 32768,
        outputTokens: 512,
        turnDurationMs: 4000,
      };
      const contextWindow = 131072; // 128K window

      const metrics = resolveContextUsageMetrics(turn, contextWindow, { isSubmitting: false });

      expect(metrics).toBeDefined();
      // 32768 / 131072 = 25%
      expect(metrics.contextUsagePercentage).toBe(25);
      expect(metrics.contextUsageDebugInfo).toEqual({
        inputTokens: 32768,
        contextWindow: 131072,
        calculationMode: "turn_completed_aggregate",
        generationSpeed: expect.any(Number),
      } satisfies ContextUsageDebugInfo);
      expect(metrics.contextUsageDebugInfo.generationSpeed).toBeCloseTo(128, 2);
    });

    it("当 contextWindow 无效或 <= 0 时，contextUsagePercentage 安全退化为 0", () => {
      const turn: TurnTraceRecord = {
        turnId: "turn-diag-2",
        status: "completed",
        inputTokens: 1000,
        outputTokens: 100,
        turnDurationMs: 1000,
      };

      const metrics = resolveContextUsageMetrics(turn, 0, { isSubmitting: false });
      expect(metrics.contextUsagePercentage).toBe(0);
      expect(metrics.contextUsageDebugInfo.contextWindow).toBe(0);
    });
  });
});
