import type { TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";

/**
 * 非响应式 trace 投影层：签名化 memo 缓存 turn timeline 归一化结果。
 *
 * 设计约束（PA-086，3 路对抗审核采纳）：
 * - 不复制源数据（活跃 turn 别名同数组，就地变更可见），只缓存归一化结果。
 * - memo key = traceTimeline 引用 + updatedAt：引用/时间戳稳定则命中，
 *   活跃 turn 就地更新（updateActiveModelTraceFromAssistant 触发 updatedAt 变化）自动失效。
 * - 模块级非响应式存储：不进 Pinia state，不参与 devtools 序列化与持久化。
 */

export function canonicalTraceTimelineKind(kind: TraceTimelineEntry["kind"]): TraceTimelineEntry["kind"] {
  switch (kind) {
    case "context":
      return "build_context";
    case "model":
      return "call_model";
    case "tool":
      return "call_tool";
    case "return":
      return "return_result";
    default:
      return kind;
  }
}

interface TurnTimelineMemoEntry {
  ref: TraceTimelineEntry[] | null | undefined;
  updatedAt?: number;
  result: TraceTimelineEntry[];
}

const turnTimelineMemo = new Map<string, TurnTimelineMemoEntry>();

/** 归一化 turn timeline（prepare_retrieval 剔除、return_result 折叠进 model 条目）。 */
export function computeTurnTimeline(turn: TurnTraceRecord): TraceTimelineEntry[] {
  if (turn.traceTimeline?.length) {
    const normalized: TraceTimelineEntry[] = [];
    let lastModelIndex = -1;
    for (const entry of turn.traceTimeline) {
      const kind = canonicalTraceTimelineKind(entry.kind);
      if (kind === "prepare_retrieval") {
        continue;
      }
      if (kind !== "return_result") {
        normalized.push({ ...entry, kind });
        if (kind === "call_model") {
          lastModelIndex = normalized.length - 1;
        }
        continue;
      }

      if (lastModelIndex === -1) {
        normalized.push({
          ...entry,
          id: `model-${entry.sequence}`,
          kind: "call_model",
          label: "CALL MODEL #1",
          text: entry.state === "completed" ? entry.text ?? null : null
        });
        lastModelIndex = normalized.length - 1;
        continue;
      }

      const modelEntry = normalized[lastModelIndex];
      normalized[lastModelIndex] = {
        ...modelEntry,
        kind: "call_model",
        state: entry.state ?? modelEntry.state,
        text: entry.state === "completed" ? entry.text ?? modelEntry.text ?? null : modelEntry.text ?? null,
        reasoningContent: entry.reasoningContent ?? modelEntry.reasoningContent ?? null,
        fallbackReason: entry.fallbackReason ?? modelEntry.fallbackReason ?? null,
        error: entry.error ?? modelEntry.error ?? null,
        inputTokens: entry.inputTokens ?? modelEntry.inputTokens ?? null,
        cacheHitInputTokens: entry.cacheHitInputTokens ?? modelEntry.cacheHitInputTokens ?? null,
        reasoningTokens: entry.reasoningTokens ?? modelEntry.reasoningTokens ?? null,
        outputTokens: entry.outputTokens ?? modelEntry.outputTokens ?? null,
        totalTokens: entry.totalTokens ?? modelEntry.totalTokens ?? null,
        firstTokenLatencyMs: entry.firstTokenLatencyMs ?? modelEntry.firstTokenLatencyMs ?? null,
        turnDurationMs: entry.turnDurationMs ?? modelEntry.turnDurationMs ?? null
      };
    }
    return normalized;
  }

  return [];
}

/** 签名化 memo 读取：引用与 updatedAt 均未变则返回缓存结果，否则重算并写回。 */
export function turnTimeline(turn: TurnTraceRecord): TraceTimelineEntry[] {
  const cached = turnTimelineMemo.get(turn.turnId);
  if (cached && cached.ref === turn.traceTimeline && cached.updatedAt === turn.updatedAt) {
    return cached.result;
  }

  const result = computeTurnTimeline(turn);
  turnTimelineMemo.set(turn.turnId, {
    ref: turn.traceTimeline,
    updatedAt: turn.updatedAt,
    result
  });
  return result;
}

/** 会话切换/组件卸载时清理 memo，避免跨会话陈旧引用。 */
export function clearTraceProjectionMemo(): void {
  turnTimelineMemo.clear();
}