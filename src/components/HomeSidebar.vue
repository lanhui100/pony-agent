<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  AlertTriangle,
  AudioLines,
  Check,
  ChevronRight,
  Circle,
  Clock3,
  Copy,
  FileText,
  Image as ImageIcon,
  LoaderCircle,
  Orbit,
  ScanSearch,
  Video,
  Wrench
} from "lucide-vue-next";
import { extractActiveTaskFocus } from "@/types/runtime";
import type {
  BuildContextObservation,
  ProviderCallCacheRecord,
  RunControlAuditSummary,
  ToolActivity,
  TraceStep,
  TraceTimelineEntry,
  TurnTraceRecord
} from "@/types/runtime";
import { useRuntimeStore } from "@/stores/runtime";
import { useProviderStore } from "@/stores/providers";
import ScrollArea from "@/components/ui/ScrollArea.vue";

type DetailRowTone = "default" | "muted" | "warning" | "danger";
type InputKind = "text" | "image" | "video" | "audio";

type DetailRow = {
  label: string;
  value: string;
  multiline?: boolean;
  tone?: DetailRowTone;
  inputKind?: InputKind;
  expandable?: boolean;
};

type TraceDetailSection = {
  id: string;
  label: string;
  content: string;
  summary?: string;
  tone?: DetailRowTone;
  kind?: "default" | "tool" | "model";
  toolStatus?: ToolActivity["status"];
  durationText?: string;
};

type TimelineDetailRow = DetailRow;

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();

const {
  availableTools,
  activeTurnId: runtimeActiveTurnId,
  error,
  fallbackReason,
  firstTokenLatencyMs,
  inputTokens,
  isSubmitting,
  latestRunControlAuditSummary,
  messages,
  outputTokens,
  phase,
  phaseLabel,
  providerMode,
  providerModel,
  providerName,
  providerProtocol,
  retrievedContext,
  sessionError,
  sessionId,
  sessionOperation,
  toolActivities,
  totalTokens,
  traceSteps,
  traceTimeline,
  turnTraceHistory
} = storeToRefs(runtimeStore);

const activePanel = ref<"trace" | "tools" | "">("trace");
const activeTurnId = ref("");
const activeTraceStepKey = ref("");
const activeTraceDetailKey = ref("");
const copiedKey = ref("");
const expandedResultKeys = ref<string[]>([]);
let copiedTimer: number | null = null;

const displayModel = computed(() => {
  const provider = providerName.value?.trim();
  const model = providerModel.value?.trim();

  if (provider && model) {
    return `${provider}/${model}`;
  }

  return model || provider || "";
});

const retrievedSessionContext = computed(() => retrievedContext.value?.sessionContext ?? null);
const retrievedRunState = computed(() => retrievedContext.value?.runState ?? null);
const retrievedLongTermMemory = computed(() => retrievedContext.value?.longTermMemory ?? null);
const retrievedRunPhase = computed(
  () =>
    retrievedRunState.value?.phase?.trim() ||
    retrievedRunState.value?.executionCheckpointPhase?.trim() ||
    retrievedRunState.value?.executionCheckpointStatus?.trim() ||
    ""
);
const longTermMemoryEntries = computed(() => retrievedLongTermMemory.value?.entries ?? []);
const longTermMemoryPreviewEntries = computed(() => longTermMemoryEntries.value.slice(0, 3));
const retrievedActiveTaskFocus = computed(() => extractActiveTaskFocus(longTermMemoryEntries.value)?.trim() ?? "");
const controlStatusSummary = computed(() => buildControlStatusSummary(latestRunControlAuditSummary.value));
const sessionStatusSummary = computed(() => {
  if (sessionOperation.value === "initializing") {
    return "正在加载最近对话…";
  }

  if (sessionOperation.value === "switching") {
    return "正在切换对话…";
  }

  if (sessionOperation.value === "deleting") {
    return "正在删除对话并刷新会话状态…";
  }

  if (sessionError.value?.trim()) {
    return sessionError.value.trim();
  }

  if (error.value?.trim()) {
    return phaseLabel.value === "失败" ? error.value.trim() : `最近错误：${error.value.trim()}`;
  }

  if (isSubmitting.value) {
    return "当前轮次正在执行。";
  }

  return "";
});

function turnTraceSortKey(turn: TurnTraceRecord) {
  return turn.updatedAt
    ?? turn.emittedAtMs
    ?? turn.sequence
    ?? 0;
}

function compareTurnTraceOrder(left: TurnTraceRecord, right: TurnTraceRecord) {
  const leftKey = turnTraceSortKey(left);
  const rightKey = turnTraceSortKey(right);
  if (leftKey !== rightKey) {
    return leftKey - rightKey;
  }

  return left.turnId.localeCompare(right.turnId);
}

const liveTraceTurn = computed<TurnTraceRecord | null>(() => {
  const turnId = runtimeActiveTurnId.value?.trim() || "";
  if (!isSubmitting.value || !turnId || traceTimeline.value.length === 0) {
    return null;
  }

  const latestUserMessage = [...messages.value]
    .reverse()
    .find((message) => message.turnId === turnId && message.role === "user");

  return {
    turnId,
    title: latestUserMessage?.content?.trim() || "当前执行中",
    phase: phase.value,
    traceSteps: traceSteps.value,
    traceTimeline: traceTimeline.value,
    toolActivities: toolActivities.value,
    providerName: providerName.value,
    providerProtocol: providerProtocol.value,
    providerModel: providerModel.value,
    providerMode: providerMode.value,
    fallbackReason: fallbackReason.value,
    error: error.value,
    inputTokens: inputTokens.value,
    outputTokens: outputTokens.value,
    totalTokens: totalTokens.value,
    firstTokenLatencyMs: firstTokenLatencyMs.value,
    updatedAt: Date.now()
  };
});

const orderedTurnTraces = computed(() => {
  const turns = [...turnTraceHistory.value];
  const activeTurn = liveTraceTurn.value;
  if (activeTurn) {
    const existingIndex = turns.findIndex((turn) => turn.turnId === activeTurn.turnId);
    if (existingIndex >= 0) {
      turns[existingIndex] = activeTurn;
    } else {
      turns.push(activeTurn);
    }
  }
  return turns.sort(compareTurnTraceOrder);
});
const latestTurn = computed(() => orderedTurnTraces.value[orderedTurnTraces.value.length - 1] ?? null);
const latestTurnId = computed(() => orderedTurnTraces.value[orderedTurnTraces.value.length - 1]?.turnId ?? "");
const currentContextWindowTokens = computed(
  () => retrievedSessionContext.value?.contextWindowTokens ?? providerStore.currentModel?.capabilities?.contextWindowTokens ?? null
);
const sessionTurnCount = computed(() => orderedTurnTraces.value.length);
const sessionModelCallCount = computed(() =>
  orderedTurnTraces.value.reduce(
    (sum, turn) => sum + turnTimeline(turn).filter((entry) => canonicalTraceTimelineKind(entry.kind) === "call_model").length,
    0
  )
);
const sessionToolCallCount = computed(() =>
  orderedTurnTraces.value.reduce(
    (sum, turn) => sum + turnTimeline(turn).filter((entry) => canonicalTraceTimelineKind(entry.kind) === "call_tool").length,
    0
  )
);
const sessionInputTokensTotal = computed(() =>
  orderedTurnTraces.value.reduce((sum, turn) => sum + (turn.inputTokens ?? 0), 0)
);
const sessionCacheHitTokensTotal = computed(() =>
  orderedTurnTraces.value.reduce((sum, turn) => sum + (cacheHitInputTokens(turn) ?? 0), 0)
);
const sessionOutputTokensTotal = computed(() =>
  orderedTurnTraces.value.reduce((sum, turn) => sum + (turn.outputTokens ?? 0), 0)
);
const sessionCacheHitRatio = computed(() => {
  if (sessionInputTokensTotal.value <= 0) {
    return "";
  }

  return `${((sessionCacheHitTokensTotal.value / sessionInputTokensTotal.value) * 100).toFixed(1)}%`;
});

function formatDuration(durationSeconds?: number | null) {
  if (durationSeconds == null) {
    return "";
  }

  return durationSeconds < 1 ? `${Math.round(durationSeconds * 1000)} ms` : `${durationSeconds.toFixed(2)} s`;
}

function buildControlStatusSummary(summary: RunControlAuditSummary | null) {
  const actionSummary = summary?.actionEvidenceSummary ?? null;
  if (!actionSummary?.summary?.trim()) {
    return "";
  }

  const details = [actionSummary.commandKind, actionSummary.boundary, actionSummary.resultKind]
    .filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    .map((value) => value.trim());
  const suffix = details.length ? ` · ${details.join(" / ")}` : "";
  return `${actionSummary.summary.trim()}${suffix}`;
}

function traceStateIcon(state: TraceStep["state"]) {
  if (state === "completed") {
    return Check;
  }

  if (state === "active") {
    return LoaderCircle;
  }

  if (state === "error") {
    return AlertTriangle;
  }

  return Circle;
}

function turnTimeline(turn: TurnTraceRecord) {
  if (turn.traceTimeline?.length) {
    const normalized: TraceTimelineEntry[] = [];
    for (const entry of turn.traceTimeline) {
      const kind = canonicalTraceTimelineKind(entry.kind);
      if (kind === "prepare_retrieval") {
        continue;
      }
      if (kind !== "return_result") {
        normalized.push({ ...entry, kind });
        continue;
      }

      const reverseModelIndex = [...normalized].reverse().findIndex((candidate) => candidate.kind === "call_model");
      if (reverseModelIndex === -1) {
        normalized.push({
          ...entry,
          id: `model-${entry.sequence}`,
          kind: "call_model",
          label: "CALL MODEL #1",
          text: entry.state === "completed" ? entry.text ?? null : null
        });
        continue;
      }

      const modelIndex = normalized.length - 1 - reverseModelIndex;
      const modelEntry = normalized[modelIndex];
      normalized[modelIndex] = {
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

function turnStateIcon(turn: TurnTraceRecord) {
  if (turn.phase === "failed" || turn.error) {
    return AlertTriangle;
  }

  if (turn.phase === "calling_model" || turn.phase === "calling_tool") {
    return LoaderCircle;
  }

  if (turn.phase === "completed") {
    return Check;
  }

  return Orbit;
}

function turnMeta(turn: TurnTraceRecord) {
  return buildTurnAggregateMetrics(turn).join(" · ");
}

function detailText(turn: TurnTraceRecord) {
  if (turn.error) {
    return turn.error;
  }

  if (turn.fallbackReason) {
    return turn.fallbackReason;
  }

  return "";
}

function shouldShowTurnDetailText(turn: TurnTraceRecord) {
  return !turnMeta(turn) && !!detailText(turn);
}

function rowToneClass(tone: DetailRowTone = "default") {
  if (tone === "danger") {
    return "text-rose-700";
  }

  if (tone === "warning") {
    return "text-amber-800";
  }

  if (tone === "muted") {
    return "text-stone-500";
  }

  return "text-stone-700";
}

function pushRow(rows: DetailRow[], label: string, value?: string | number | null, options: Omit<DetailRow, "label" | "value"> = {}) {
  if (value == null) {
    return;
  }

  const normalized = String(value).trim();
  if (!normalized) {
    return;
  }

  rows.push({
    label,
    value: normalized,
    ...options
  });
}

function buildContextRows(buildContextObservation?: BuildContextObservation | null) {
  const rows: DetailRow[] = [];

  if (!buildContextObservation) {
    return rows;
  }

  pushRow(rows, "请求格式", buildContextObservation.requestFormat);
  pushRow(rows, "消息数", buildContextObservation.messageCount);
  pushRow(rows, "图片数", buildContextObservation.imageCount);
  pushRow(rows, "工具数", buildContextObservation.toolCount);
  pushRow(rows, "温度", buildContextObservation.temperature);
  pushRow(rows, "最大输出", buildContextObservation.maxOutputTokens);

  return rows;
}

function buildContextText(buildContextObservation: BuildContextObservation | null | undefined, key: keyof BuildContextObservation) {
  const value = buildContextObservation?.[key];
  return typeof value === "string" ? value.trim() : "";
}

function inputKindIcon(kind: InputKind) {
  if (kind === "image") {
    return ImageIcon;
  }

  if (kind === "video") {
    return Video;
  }

  if (kind === "audio") {
    return AudioLines;
  }

  return FileText;
}

function toolPanelKey(name: string) {
  return `tool:${name}`;
}

function turnStepKey(turnId: string, stepId: string) {
  return `${turnId}:${stepId}`;
}

function traceCopyKey(turnId: string, stepId: string) {
  return `trace:${turnId}:${stepId}`;
}

function traceDetailKey(turnId: string, stepId: string, detailId: string) {
  return `${turnId}:${stepId}:${detailId}`;
}

function expandedResultKey(turnId: string, stepId: string, label: string) {
  return `${turnId}:${stepId}:${label}`;
}

function previewInline(text: string, maxChars = 72) {
  const normalized = text.replace(/\s+/g, " ").trim();
  if (normalized.length <= maxChars) {
    return normalized;
  }

  return `${normalized.slice(0, maxChars)}...`;
}

function formatDurationMs(durationMs?: number | null) {
  if (durationMs == null) {
    return "";
  }

  return durationMs < 1000 ? `${Math.round(durationMs)} ms` : `${(durationMs / 1000).toFixed(2)} s`;
}

function formatTightCompactDurationMs(durationMs?: number | null) {
  if (durationMs == null) {
    return "";
  }

  return durationMs < 1000 ? `${Math.round(durationMs)}ms` : `${(durationMs / 1000).toFixed(1)}s`;
}

function formatInteger(value?: number | null) {
  return value == null ? "" : value.toLocaleString("zh-CN");
}

function formatContextUsage(inputTokens?: number | null, contextWindowTokens?: number | null) {
  if (inputTokens == null) {
    return "";
  }

  const used = formatInteger(inputTokens);
  if (contextWindowTokens == null || contextWindowTokens <= 0) {
    return used;
  }

  return `${used} / ${formatInteger(contextWindowTokens)} (${((inputTokens / contextWindowTokens) * 100).toFixed(1)}%)`;
}

function readNumericValue(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readNestedNumericValue(source: unknown, paths: string[][]) {
  if (!source || typeof source !== "object") {
    return null;
  }

  for (const path of paths) {
    let current: unknown = source;

    for (const segment of path) {
      if (!current || typeof current !== "object") {
        current = null;
        break;
      }

      current = (current as Record<string, unknown>)[segment];
    }

    const resolved = readNumericValue(current);
    if (resolved != null) {
      return resolved;
    }
  }

  return null;
}

function cacheHitInputTokens(turn: TurnTraceRecord) {
  return readNestedNumericValue(turn, [
    ["cacheHitInputTokens"],
    ["cache_hit_input_tokens"],
    ["promptCacheHitTokens"],
    ["prompt_cache_hit_tokens"],
    ["cachedInputTokens"],
    ["cacheReadInputTokens"],
    ["inputCachedTokens"],
    ["cachedTokens"],
    ["inputTokensDetails", "cachedTokens"],
    ["input_tokens_details", "cached_tokens"],
    ["promptTokensDetails", "cachedTokens"],
    ["prompt_tokens_details", "cached_tokens"],
    ["usage", "input_tokens_details", "cached_tokens"],
    ["usage", "prompt_tokens_details", "cached_tokens"]
  ]);
}

function reasoningTokens(turn: TurnTraceRecord) {
  return readNestedNumericValue(turn, [
    ["reasoningTokens"],
    ["completionTokensDetails", "reasoningTokens"],
    ["completion_tokens_details", "reasoning_tokens"],
    ["outputTokensDetails", "reasoningTokens"],
    ["output_tokens_details", "reasoning_tokens"],
    ["usage", "completion_tokens_details", "reasoning_tokens"],
    ["usage", "output_tokens_details", "reasoning_tokens"]
  ]);
}

function tokenSpeed(turn: TurnTraceRecord) {
  if (turn.outputTokens == null || turn.turnDurationMs == null) {
    return null;
  }

  const durationMs = Math.max(turn.turnDurationMs, 1);
  const tokensPerSecond = turn.outputTokens / (durationMs / 1000);
  return Number.isFinite(tokensPerSecond) ? tokensPerSecond : null;
}

function hasTrueFirstTokenLatency(record: ProviderCallCacheRecord | null | undefined, entry: TraceTimelineEntry) {
  if (record) {
    return record.latencyKind === "provider_stream";
  }

  return entry.firstTokenLatencyMs != null;
}

function effectiveFirstTokenLatencyMs(entry: TraceTimelineEntry, record: ProviderCallCacheRecord | null | undefined) {
  if (!hasTrueFirstTokenLatency(record, entry)) {
    return null;
  }

  const latency = record?.firstTokenLatencyMs ?? entry.firstTokenLatencyMs ?? null;
  if (latency == null) {
    return null;
  }

  if (entry.turnDurationMs != null && latency >= entry.turnDurationMs) {
    return null;
  }

  return latency;
}

function activeGenerationDurationMs(entry: TraceTimelineEntry, _record: ProviderCallCacheRecord | null | undefined) {
  if (entry.turnDurationMs == null) {
    return null;
  }

  return Math.max(entry.turnDurationMs, 1);
}

function formatEntryTokenSpeed(entry: TraceTimelineEntry, record?: ProviderCallCacheRecord | null) {
  if (entry.outputTokens == null || entry.turnDurationMs == null) {
    return "";
  }

  const activeGenerationMs = activeGenerationDurationMs(entry, record);
  if (activeGenerationMs == null) {
    return "";
  }
  const value = entry.outputTokens / (activeGenerationMs / 1000);
  return Number.isFinite(value) ? `${value.toFixed(1)} token/s` : "";
}

function average(values: number[]) {
  if (!values.length) {
    return null;
  }

  const total = values.reduce((sum, value) => sum + value, 0);
  return total / values.length;
}

function timelineCallModelIndex(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  return turnTimeline(turn)
    .filter((candidate) => canonicalTraceTimelineKind(candidate.kind) === "call_model")
    .findIndex((candidate) => candidate.id === entry.id);
}

function timelineProviderCallRecord(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  if (canonicalTraceTimelineKind(entry.kind) !== "call_model") {
    return null;
  }

  const index = timelineCallModelIndex(turn, entry);
  return index >= 0 ? turn.providerCallRecords?.[index] ?? null : null;
}

function timelineMetricEntry(turn: TurnTraceRecord, entry: TraceTimelineEntry, options: { allowTurnFallback?: boolean } = {}): TraceTimelineEntry {
  const record = timelineProviderCallRecord(turn, entry);
  const allowTurnFallback = options.allowTurnFallback ?? true;
  const callModelEntries = turnTimeline(turn).filter((candidate) => canonicalTraceTimelineKind(candidate.kind) === "call_model");
  const useTurnFallback = allowTurnFallback && callModelEntries.length === 1 && callModelEntries[0]?.id === entry.id;

  return {
    ...entry,
    inputTokens: entry.inputTokens ?? record?.inputTokens ?? (useTurnFallback ? turn.inputTokens ?? null : null),
    cacheHitInputTokens:
      entry.cacheHitInputTokens ?? record?.cacheHitInputTokens ?? (useTurnFallback ? cacheHitInputTokens(turn) ?? null : null),
    reasoningTokens:
      entry.reasoningTokens ?? record?.reasoningTokens ?? (useTurnFallback ? reasoningTokens(turn) ?? null : null),
    outputTokens: entry.outputTokens ?? record?.outputTokens ?? (useTurnFallback ? turn.outputTokens ?? null : null),
    totalTokens: entry.totalTokens ?? record?.totalTokens ?? (useTurnFallback ? turn.totalTokens ?? null : null),
    firstTokenLatencyMs: effectiveFirstTokenLatencyMs(
      {
        ...entry,
        firstTokenLatencyMs: record?.firstTokenLatencyMs ?? entry.firstTokenLatencyMs ?? (useTurnFallback ? turn.firstTokenLatencyMs ?? null : null),
        turnDurationMs: record?.turnDurationMs ?? entry.turnDurationMs ?? (useTurnFallback ? turn.turnDurationMs ?? null : null)
      },
      record
    ),
    turnDurationMs: record?.turnDurationMs ?? entry.turnDurationMs ?? (useTurnFallback ? turn.turnDurationMs ?? null : null)
  };
}

function traceStateLabel(state: TraceTimelineEntry["state"]) {
  switch (state) {
    case "completed":
      return "已完成";
    case "active":
      return "进行中";
    case "error":
      return "失败";
    case "pending":
      return "待执行";
    default:
      return "";
  }
}

function formatProviderModel(providerName?: string | null, providerModel?: string | null) {
  const provider = providerName?.trim();
  const model = providerModel?.trim();
  if (provider && model) {
    return `${provider}/${model}`;
  }

  return provider || model || "";
}

function buildTurnAggregateMetrics(turn: TurnTraceRecord) {
  const callModelEntries = turnTimeline(turn).filter((entry) => canonicalTraceTimelineKind(entry.kind) === "call_model");
  const perCallMetrics = callModelEntries.map((entry) => timelineMetricEntry(turn, entry, { allowTurnFallback: false }));
  const hasPerCallMetrics = perCallMetrics.some((entry) =>
    entry.inputTokens != null
    || entry.cacheHitInputTokens != null
    || entry.outputTokens != null
    || entry.firstTokenLatencyMs != null
    || entry.turnDurationMs != null
  );

  const inputs = hasPerCallMetrics ? perCallMetrics.map((entry) => entry.inputTokens).filter((value): value is number => value != null) : [];
  const caches = hasPerCallMetrics ? perCallMetrics.map((entry) => entry.cacheHitInputTokens).filter((value): value is number => value != null) : [];
  const outputs = hasPerCallMetrics ? perCallMetrics.map((entry) => entry.outputTokens).filter((value): value is number => value != null) : [];
  const latencies = hasPerCallMetrics ? perCallMetrics.map((entry) => entry.firstTokenLatencyMs).filter((value): value is number => value != null) : [];
  const generationDurations = hasPerCallMetrics
    ? callModelEntries
      .map((entry, index) => activeGenerationDurationMs(perCallMetrics[index]!, timelineProviderCallRecord(turn, entry)))
      .filter((value): value is number => value != null)
    : [];

  const metrics: string[] = [];
  const inputTotal = inputs.length ? inputs.reduce((sum, value) => sum + value, 0) : turn.inputTokens ?? null;
  const cacheTotal = caches.length ? caches.reduce((sum, value) => sum + value, 0) : cacheHitInputTokens(turn);
  const outputTotal = outputs.length ? outputs.reduce((sum, value) => sum + value, 0) : turn.outputTokens ?? null;
  const speedAverage = outputTotal != null && generationDurations.length
    ? outputTotal / (generationDurations.reduce((sum, value) => sum + value, 0) / 1000)
    : tokenSpeed(turn);
  const latencyAverage = latencies.length ? average(latencies) : turn.firstTokenLatencyMs ?? null;

  if (inputTotal != null) {
    metrics.push(`输入 ${formatInteger(inputTotal)}`);
  }
  if (cacheTotal != null) {
    metrics.push(`缓存 ${formatInteger(cacheTotal)}`);
  }
  if (outputTotal != null) {
    metrics.push(`输出 ${formatInteger(outputTotal)}`);
  }
  if (speedAverage != null) {
    metrics.push(`速度 ${speedAverage.toFixed(1)} token/s`);
  }
  if (latencyAverage != null) {
    metrics.push(`延时 ${Math.round(latencyAverage)} ms`);
  }

  return metrics;
}

function canonicalTraceTimelineKind(kind: TraceTimelineEntry["kind"]) {
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

function timelineEntryStats(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  if (canonicalTraceTimelineKind(entry.kind) !== "call_model") {
    return [];
  }

  const metricEntry = timelineMetricEntry(turn, entry);
  const stats: string[] = [];
  if (metricEntry.inputTokens != null) {
    stats.push(`输入 ${formatInteger(metricEntry.inputTokens)}`);
  }
  if (metricEntry.cacheHitInputTokens != null) {
    stats.push(`缓存 ${formatInteger(metricEntry.cacheHitInputTokens)}`);
  }
  if (metricEntry.outputTokens != null) {
    stats.push(`输出 ${formatInteger(metricEntry.outputTokens)}`);
  }
  const tokenSpeed = formatEntryTokenSpeed(metricEntry);
  if (tokenSpeed) {
    stats.push(`速度 ${tokenSpeed}`);
  }
  if (metricEntry.firstTokenLatencyMs != null) {
    stats.push(`延时 ${metricEntry.firstTokenLatencyMs} ms`);
  }
  return stats;
}

function timelineDurationText(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const kind = canonicalTraceTimelineKind(entry.kind);
  if (kind === "call_tool") {
    const totalSeconds = (entry.toolActivities ?? []).reduce((sum, tool) => sum + (tool.durationSeconds ?? 0), 0);
    return totalSeconds > 0 ? formatDuration(totalSeconds) : "";
  }

  if (kind === "call_model") {
    const metricEntry = timelineMetricEntry(turn, entry);
    if (metricEntry.turnDurationMs != null) {
      return formatDurationMs(metricEntry.turnDurationMs);
    }
  }

  return "";
}

function timelineEntryIndex(turn: TurnTraceRecord, entryId: string) {
  return turnTimeline(turn).findIndex((candidate) => candidate.id === entryId);
}

function callModelOutputToolEntries(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  if (canonicalTraceTimelineKind(entry.kind) !== "call_model") {
    return [];
  }

  const timeline = turnTimeline(turn);
  const entryIndex = timelineEntryIndex(turn, entry.id);
  if (entryIndex === -1) {
    return [];
  }

  const outputEntries: TraceTimelineEntry[] = [];
  for (let index = entryIndex + 1; index < timeline.length; index += 1) {
    const candidate = timeline[index]!;
    const kind = canonicalTraceTimelineKind(candidate.kind);
    if (kind === "call_model") {
      break;
    }
    if (kind === "call_tool") {
      outputEntries.push(candidate);
    }
  }

  return outputEntries;
}

function hasCallModelToolOutputs(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  return callModelOutputToolEntries(turn, entry).length > 0;
}

function shouldShowCallModelOutput(entry: TraceTimelineEntry) {
  return canonicalTraceTimelineKind(entry.kind) === "call_model" && entry.state !== "active" && entry.state !== "pending";
}

function shouldShowCallModelReasoning(entry: TraceTimelineEntry) {
  return canonicalTraceTimelineKind(entry.kind) === "call_model" && entry.state !== "active" && entry.state !== "pending";
}

function timelinePreviewText(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const kind = canonicalTraceTimelineKind(entry.kind);
  if (kind === "call_tool") {
    return "";
  }

  if (kind === "call_model") {
    return (
      entry.error?.trim()
      || entry.fallbackReason?.trim()
      || ""
    );
  }

  if (entry.text?.trim()) {
    return entry.text.trim();
  }

  if (entry.reasoningContent?.trim()) {
    return entry.reasoningContent.trim();
  }

  if (entry.fallbackReason?.trim()) {
    return entry.fallbackReason.trim();
  }

  if (entry.error?.trim()) {
    return entry.error.trim();
  }

  return "";
}

function buildTimelineRows(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const rows: TimelineDetailRow[] = [];
  const kind = canonicalTraceTimelineKind(entry.kind);

  if (kind === "input") {
    pushRow(rows, "输入", timelinePreviewText(turn, entry), { multiline: true });
    return rows;
  }

  if (kind === "build_context") {
    buildContextRows(entry.buildContextObservation ?? turn.buildContextObservation).forEach((row) => rows.push(row));
    pushRow(rows, "请求目标", entry.providerRequestedName ?? turn.providerRequestedName);
    pushRow(rows, "Provider", entry.providerName);
    pushRow(rows, "Protocol", entry.providerProtocol);
    pushRow(rows, "Model", entry.providerModel);
    pushRow(rows, "模式", entry.providerMode);
    pushRow(
      rows,
      "观测说明",
      entry.buildContextObservation ?? turn.buildContextObservation
        ? "这里展示的是本轮真正发给模型的请求，不是 retrieval state 的替身。"
        : "当前还没有可展示的 request observation。",
      { multiline: true, tone: "muted" }
    );
    return rows;
  }

  if (kind === "call_model") {
    const metricEntry = timelineMetricEntry(turn, entry);
    pushRow(rows, "阶段", traceStateLabel(entry.state));
    pushRow(rows, "模型", formatProviderModel(entry.providerName, entry.providerModel));
    pushRow(rows, "输入", metricEntry.inputTokens);
    pushRow(rows, "缓存", metricEntry.cacheHitInputTokens);
    pushRow(rows, "输出", metricEntry.outputTokens);
    pushRow(rows, "速度", formatEntryTokenSpeed(metricEntry) || null);
    pushRow(rows, "延时", metricEntry.firstTokenLatencyMs != null ? `${metricEntry.firstTokenLatencyMs} ms` : null);
    pushRow(rows, "耗时", metricEntry.turnDurationMs != null ? formatDurationMs(metricEntry.turnDurationMs) : null);
    pushRow(rows, "错误", entry.error, { multiline: true, tone: "danger" });
    return rows;
  }

  if (kind === "call_tool") {
    pushRow(rows, "耗时", timelineDurationText(turn, entry));
    pushRow(rows, "错误", entry.error, { multiline: true, tone: "danger" });
    return rows;
  }

  return rows;
}

function buildTimelineDetailSections(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const sections: TraceDetailSection[] = [];
  const kind = canonicalTraceTimelineKind(entry.kind);

  if (kind === "input") {
    return sections;
  }

  if (kind === "build_context") {
    const buildContextObservation = entry.buildContextObservation ?? turn.buildContextObservation;
    const contextSections: Array<[string, string, string]> = [
      ["stable", "稳定前缀", buildContextText(buildContextObservation, "stablePrefixText")],
      ["semi", "半稳定上下文", buildContextText(buildContextObservation, "semiStableContextText")],
      ["volatile", "本轮输入", buildContextText(buildContextObservation, "volatileInputText")],
      ["messages", "最终请求消息", buildContextText(buildContextObservation, "requestMessagesText")],
      ["tools", "工具定义", buildContextText(buildContextObservation, "toolDefinitionsText")]
    ];

    for (const [id, label, content] of contextSections) {
      if (!content) {
        continue;
      }

      sections.push({
        id,
        label,
        content,
        summary: previewInline(content)
      });
    }

    return sections;
  }

  if (kind === "call_tool") {
    for (const activity of entry.toolActivities ?? []) {
      sections.push({
        id: activity.id,
        label: activity.name,
        content: buildToolMessageDetail(activity),
        summary: activity.summary,
        kind: "tool",
        toolStatus: activity.status,
        durationText: formatDuration(activity.durationSeconds)
      });
    }
    return sections;
  }

  for (const [toolEntryIndex, toolEntry] of callModelOutputToolEntries(turn, entry).entries()) {
    const activities = toolEntry.toolActivities ?? [];
    if (!activities.length) {
      const fallbackContent = [toolEntry.text?.trim(), toolEntry.error?.trim()].filter(Boolean).join("\n\n");
      if (fallbackContent) {
        sections.push({
          id: `tool-output-${toolEntry.id}`,
          label: `工具调用输出 #${toolEntryIndex + 1}`,
          content: fallbackContent,
          summary: previewResult(fallbackContent),
          kind: "tool",
          toolStatus: "planned"
        });
      }
      continue;
    }

    for (const activity of activities) {
      sections.push({
        id: `tool-output-${toolEntry.id}-${activity.id}`,
        label: activity.name,
        content: buildToolMessageDetail(activity),
        summary: activity.summary,
        kind: "tool",
        toolStatus: activity.status,
        durationText: formatDuration(activity.durationSeconds)
      });
    }
  }

  const reasoningContent = shouldShowCallModelReasoning(entry) ? entry.reasoningContent?.trim() || "" : "";
  if (reasoningContent) {
    sections.push({
      id: "reasoning",
      label: "思考链",
      content: reasoningContent,
      summary: previewResult(reasoningContent),
      kind: "model"
    });
  }

  const outputContent = shouldShowCallModelOutput(entry) ? entry.text?.trim() || "" : "";
  if (outputContent) {
    sections.push({
      id: "assistant-output",
      label: "模型输出",
      content: outputContent,
      summary: previewResult(outputContent),
      kind: "model"
    });
  }

  return sections;
}

function buildToolMessageDetail(activity: ToolActivity) {
  const lines: string[] = [];

  if (activity.argumentsText?.trim()) {
    lines.push(`参数:\n${activity.argumentsText.trim()}`);
  }

  if (activity.resultText?.trim()) {
    lines.push(`${activity.status === "error" ? "错误" : "结果"}:\n${activity.resultText.trim()}`);
  }

  if (activity.durationSeconds != null) {
    lines.push(`耗时: ${formatDuration(activity.durationSeconds)}`);
  }

  return lines.join("\n\n");
}

function buildTimelineCopyText(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const lines = [`${entry.label}`, `状态: ${entry.state}`];
  const preview = timelinePreviewText(turn, entry);
  if (preview) {
    lines.push("", preview);
  }
  buildTimelineRows(turn, entry).forEach((row) => {
    lines.push(`${row.label}: ${row.value}`);
  });
  return lines.join("\n");
}

function turnDurationText(turn: TurnTraceRecord) {
  return turn.turnDurationMs != null ? formatTightCompactDurationMs(turn.turnDurationMs) : "";
}

function toolStatusIcon(status: ToolActivity["status"]) {
  if (status === "done") {
    return Check;
  }

  if (status === "running") {
    return LoaderCircle;
  }

  if (status === "error") {
    return AlertTriangle;
  }

  return Circle;
}

function toolStatusIconClass(status: ToolActivity["status"]) {
  if (status === "done") {
    return "text-emerald-600";
  }

  if (status === "running") {
    return "animate-spin text-stone-500";
  }

  if (status === "error") {
    return "text-rose-600";
  }

  return "text-stone-300";
}

function turnCopyKey(turnId: string) {
  return `turn:${turnId}`;
}


function buildTurnCopyText(turn: TurnTraceRecord) {
  const parts = [turn.title];
  const meta = turnMeta(turn);
  const durationText = turnDurationText(turn);

  if (meta) {
    parts.push(`指标: ${meta}`);
  }

  if (durationText) {
    parts.push(`耗时: ${durationText}`);
  }

  turnTimeline(turn).forEach((entry) => {
    parts.push(buildTimelineCopyText(turn, entry));
  });

  return parts.join("\n\n");
}

function copyText(key: string, text: string) {
  if (!text.trim()) {
    return;
  }

  void navigator.clipboard.writeText(text);
  copiedKey.value = key;

  if (copiedTimer != null) {
    window.clearTimeout(copiedTimer);
  }

  copiedTimer = window.setTimeout(() => {
    copiedKey.value = "";
    copiedTimer = null;
  }, 1400);
}

function togglePanel(panel: "trace" | "tools") {
  activePanel.value = activePanel.value === panel ? "" : panel;
}

function toggleTurn(turnId: string) {
  activeTurnId.value = activeTurnId.value === turnId ? "" : turnId;
  activeTraceStepKey.value = "";
}

function toggleTraceStep(turnId: string, stepId: string) {
  const key = `${turnId}:${stepId}`;
  activeTraceStepKey.value = activeTraceStepKey.value === key ? "" : key;
  activeTraceDetailKey.value = "";
}

function toggleTraceDetail(turnId: string, stepId: string, detailId: string) {
  const key = traceDetailKey(turnId, stepId, detailId);
  activeTraceDetailKey.value = activeTraceDetailKey.value === key ? "" : key;
}

function toggleExpandedResult(key: string) {
  if (expandedResultKeys.value.includes(key)) {
    expandedResultKeys.value = expandedResultKeys.value.filter((item) => item !== key);
    return;
  }

  expandedResultKeys.value = [...expandedResultKeys.value, key];
}

function isExpandedResult(key: string) {
  return expandedResultKeys.value.includes(key);
}

function previewResult(text: string, maxChars = 240) {
  if (text.length <= maxChars) {
    return text;
  }

  return `${text.slice(0, maxChars)}...`;
}

function toolInputSummary(name: string) {
  const tool = availableTools.value.find((item) => item.name === name);
  const properties = tool?.inputSchema?.properties ?? {};
  const entries = Object.entries(properties);

  if (!entries.length) {
    return "无额外参数";
  }

  return entries
    .map(([key, schema]) => key + (schema.type ? ":" + schema.type : ""))
    .join(" 路 ");
}

function toolRequiredSummary(name: string) {
  const tool = availableTools.value.find((item) => item.name === name);
  const required = tool?.inputSchema?.required ?? [];

  if (!required.length) {
    return "无必填参数";
  }

  return required.join(" 路 ");
}

watch(
  latestTurnId,
  (turnId) => {
    if (!turnId) {
      activeTurnId.value = "";
      activeTraceStepKey.value = "";
      activeTraceDetailKey.value = "";
      return;
    }

    activeTurnId.value = turnId;
    activeTraceStepKey.value = "";
    activeTraceDetailKey.value = "";
  },
  { immediate: true }
);

watch(sessionId, () => {
  activeTurnId.value = "";
  activeTraceStepKey.value = "";
  activeTraceDetailKey.value = "";
  copiedKey.value = "";
  expandedResultKeys.value = [];
});

const orderedTurnTraceSignature = computed(() =>
  orderedTurnTraces.value
    .map((turn) => `${turn.turnId}:${turn.updatedAt}:${turn.phase}:${turn.traceTimeline?.length ?? 0}`)
    .join("|")
);

watch(orderedTurnTraceSignature, () => {
  const turns = orderedTurnTraces.value;
  if (!turns.some((turn) => turn.turnId === activeTurnId.value)) {
    activeTurnId.value = turns[turns.length - 1]?.turnId ?? "";
    activeTraceStepKey.value = "";
    activeTraceDetailKey.value = "";
  }
});
</script>

<template>
  <aside class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.6rem] border border-stone-200/70 bg-white/62">
    <ScrollArea class="min-h-0 flex-1" viewport-class="px-4 py-4">
      <div class="flex min-h-full flex-col gap-3">
        <section class="border-b border-stone-200/70 pb-4" data-open="true">
          <div class="flex w-full items-center justify-between gap-3 text-left" data-testid="status-panel-toggle">
            <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
              <ScanSearch class="h-3.5 w-3.5" />
              <span>状态</span>
            </div>
          </div>

          <section class="mt-2 space-y-2">
              <div class="grid gap-1.5 text-[12px] leading-4 text-stone-600">
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">ID</span>
                  <span class="break-words text-right text-stone-800 [overflow-wrap:anywhere]">{{ sessionId }}</span>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">阶段</span>
                  <span class="text-right text-stone-800">{{ phaseLabel }}</span>
                </div>
                <div v-if="sessionStatusSummary" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">会话状态</span>
                  <span
                    class="break-words text-right text-stone-800 [overflow-wrap:anywhere]"
                    data-testid="status-session-summary"
                  >
                    {{ sessionStatusSummary }}
                  </span>
                </div>
                <div v-if="controlStatusSummary" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">控制状态</span>
                  <span
                    class="break-words text-right text-stone-800 [overflow-wrap:anywhere]"
                    data-testid="status-control-summary"
                  >
                    {{ controlStatusSummary }}
                  </span>
                </div>
                <div v-if="displayModel" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">模型</span>
                  <span class="break-words text-right text-stone-800 [overflow-wrap:anywhere]">{{ displayModel }}</span>
                </div>
                <div v-if="providerProtocol" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">协议</span>
                  <span class="text-right text-stone-800">{{ providerProtocol }}</span>
                </div>
                <div v-if="providerMode" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">模式</span>
                  <span class="text-right text-stone-800">{{ providerMode }}</span>
                </div>
                <div v-if="latestTurn" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">上下文</span>
                  <span class="text-right text-stone-800">
                    {{ formatContextUsage(latestTurn.inputTokens, currentContextWindowTokens) || "未知" }}
                  </span>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">输入</span>
                  <span class="text-right text-stone-800">{{ formatInteger(sessionInputTokensTotal) || "0" }}</span>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">缓存命中</span>
                  <span class="text-right text-stone-800">
                    {{ formatInteger(sessionCacheHitTokensTotal) || "0" }}
                    <span v-if="sessionCacheHitRatio" class="text-stone-400">· {{ sessionCacheHitRatio }}</span>
                  </span>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">输出</span>
                  <span class="text-right text-stone-800">{{ formatInteger(sessionOutputTokensTotal) || "0" }}</span>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">轮次</span>
                  <span class="text-right text-stone-800">{{ sessionTurnCount }}</span>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">模型调用</span>
                  <span class="text-right text-stone-800">{{ sessionModelCallCount }}</span>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">工具调用</span>
                  <span class="text-right text-stone-800">{{ sessionToolCallCount }}</span>
                </div>
                <div v-if="retrievedRunPhase" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">运行阶段</span>
                  <span class="text-right text-stone-800">{{ retrievedRunPhase }}</span>
                </div>
              </div>

              <p
                v-if="retrievedActiveTaskFocus"
                class="break-words text-[11px] leading-4 text-stone-600 [overflow-wrap:anywhere]"
                data-testid="retrieved-active-task"
              >
                Active task: {{ retrievedActiveTaskFocus }}
              </p>
              <div
                v-if="longTermMemoryPreviewEntries.length"
                class="space-y-1 border-t border-stone-200/80 pt-2"
                data-testid="retrieved-memory-list"
              >
                <div class="text-[10px] uppercase tracking-[0.14em] text-stone-400">Memory entries</div>
                <div
                  v-for="entry in longTermMemoryPreviewEntries"
                  :key="`${entry.kind}:${entry.content}`"
                  class="rounded-[0.4rem] border border-stone-200/80 bg-white/80 px-2 py-1"
                >
                  <div class="break-words text-[11px] leading-4 text-stone-700 [overflow-wrap:anywhere]">
                    {{ entry.content }}
                  </div>
                  <div class="mt-0.5 text-[10px] leading-4 text-stone-400">
                    {{ entry.kind }}
                  </div>
                </div>
              </div>
              <p v-if="fallbackReason" class="break-words text-[11px] leading-4 text-amber-800 [overflow-wrap:anywhere]">
                {{ fallbackReason }}
              </p>
              <p v-if="error" class="break-words text-[11px] leading-4 text-rose-700 [overflow-wrap:anywhere]">
                {{ error }}
              </p>
          </section>
        </section>

        <section class="collapsible-shell border-b border-stone-200/60 pb-4" :data-open="activePanel === 'tools'">
          <button class="flex w-full items-center justify-between gap-3 text-left" type="button" data-testid="tools-panel-toggle" @click="togglePanel('tools')">
            <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
              <Wrench class="h-3.5 w-3.5" />
              <span>Tools</span>
            </div>
            <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': activePanel === 'tools' }" />
          </button>

          <div class="collapsible-body">
            <section class="collapsible-content mt-2 space-y-1">
              <section
                v-for="tool in availableTools"
                :key="tool.name"
                class="collapsible-shell overflow-hidden rounded-[0.35rem] px-2 py-1"
                :data-open="activeTraceStepKey === toolPanelKey(tool.name)"
              >
                <button
                  class="flex w-full items-start justify-between gap-1.5 text-left"
                  type="button"
                  @click="activeTraceStepKey = activeTraceStepKey === toolPanelKey(tool.name) ? '' : toolPanelKey(tool.name)"
                >
                  <div class="min-w-0">
                    <div class="text-[11px] font-medium text-stone-800">{{ tool.name }}</div>
                    <p class="mt-0.5 text-[11px] leading-[1.3] text-stone-500">
                      {{ tool.description }}
                    </p>
                  </div>
                  <ChevronRight
                    class="mt-0.5 h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                    :class="{ 'rotate-90': activeTraceStepKey === toolPanelKey(tool.name) }"
                  />
                </button>
                <div class="collapsible-body">
                  <div class="collapsible-content mt-1">
                    <section class="border-l border-stone-200 pl-2">
                      <div class="space-y-1 text-[10px] leading-[1.3]">
                        <div class="space-y-0.5">
                          <div class="text-stone-400">参数</div>
                          <div class="text-stone-700">{{ toolInputSummary(tool.name) }}</div>
                        </div>
                        <div class="space-y-0.5">
                          <div class="text-stone-400">必填</div>
                          <div class="text-stone-700">{{ toolRequiredSummary(tool.name) }}</div>
                        </div>
                      </div>
                    </section>
                  </div>
                </div>
              </section>
            </section>
          </div>
        </section>

        <section class="collapsible-shell mt-auto pb-1" :data-open="activePanel === 'trace'">
          <button class="flex w-full items-center justify-between gap-3 text-left" type="button" data-testid="trace-panel-toggle" @click="togglePanel('trace')">
            <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
              <Clock3 class="h-3.5 w-3.5" />
              <span>Trace</span>
            </div>
            <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': activePanel === 'trace' }" />
          </button>

          <div class="collapsible-body">
            <section class="collapsible-content mt-2 space-y-1">
              <section
                v-for="turn in orderedTurnTraces"
                :key="turn.turnId"
                class="collapsible-shell overflow-hidden border-b border-stone-200/70 py-1.5 last:border-b-0"
                :data-open="activeTurnId === turn.turnId"
              >
                <button class="flex w-full items-start justify-between gap-2 text-left" type="button" @click="toggleTurn(turn.turnId)">
                  <div class="min-w-0 space-y-0.5">
                    <div class="flex items-center gap-1.5 text-[12px] font-medium text-stone-800">
                      <component
                        :is="turnStateIcon(turn)"
                        class="h-3 w-3 shrink-0"
                        :class="{
                          'animate-spin text-stone-500': turn.phase === 'calling_model' || turn.phase === 'calling_tool',
                          'text-rose-600': turn.phase === 'failed' || !!turn.error,
                          'text-stone-700': turn.phase === 'completed' && !turn.error,
                          'text-stone-500': turn.phase !== 'failed' && turn.phase !== 'completed' && !turn.error
                        }"
                      />
                      <span class="truncate">{{ turn.title }}</span>
                    </div>
                    <div v-if="turnMeta(turn)" class="pl-[1.125rem] text-[10px] leading-[1.15] text-stone-400">
                      {{ turnMeta(turn) }}
                    </div>
                  </div>
                  <div class="flex items-center gap-1">
                    <span v-if="turnDurationText(turn)" class="shrink-0 whitespace-nowrap text-[10px] leading-[1.3] text-stone-400">
                      {{ turnDurationText(turn) }}
                    </span>
                    <button
                      class="inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                      type="button"
                      @click.stop="copyText(turnCopyKey(turn.turnId), buildTurnCopyText(turn))"
                    >
                      <component :is="copiedKey === turnCopyKey(turn.turnId) ? Check : Copy" class="h-3 w-3" />
                    </button>
                    <ChevronRight class="h-3 w-3 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': activeTurnId === turn.turnId }" />
                  </div>
                </button>

                <div class="collapsible-body">
                  <div class="collapsible-content mt-1 space-y-1 pl-4">
                    <p v-if="shouldShowTurnDetailText(turn)" class="break-words text-[10px] leading-[1.2] text-stone-500 [overflow-wrap:anywhere]">
                      {{ detailText(turn) }}
                    </p>

                    <section
                      v-for="entry in turnTimeline(turn)"
                      :key="entry.id"
                      class="collapsible-shell overflow-hidden py-0.5"
                      :data-open="activeTraceStepKey === turnStepKey(turn.turnId, entry.id)"
                    >
                      <button
                        class="flex w-full items-start justify-between gap-1.5 text-left"
                        type="button"
                        :data-testid="`trace-step-button-${entry.id}`"
                        @click="toggleTraceStep(turn.turnId, entry.id)"
                      >
                        <div class="min-w-0 space-y-0.5">
                          <div class="flex min-w-0 items-center gap-1.5 text-[11px] leading-[1.3] text-stone-700">
                            <component
                              :is="traceStateIcon(entry.state)"
                              class="h-3 w-3 shrink-0"
                              :class="{
                                'text-stone-500': entry.state === 'completed',
                                'animate-spin text-stone-500': entry.state === 'active',
                                'text-rose-600': entry.state === 'error',
                                'text-stone-300': entry.state === 'pending'
                              }"
                            />
                            <span class="truncate">{{ entry.label }}</span>
                          </div>
                          <div class="pl-[1.125rem] text-[10px] leading-[1.1] text-stone-400">
                            <div class="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
                              <span
                                v-for="stat in timelineEntryStats(turn, entry)"
                                :key="turn.turnId + '-' + entry.id + '-' + stat"
                                class="text-stone-400"
                              >
                                {{ stat }}
                              </span>
                            </div>
                            <div
                              v-if="timelinePreviewText(turn, entry) && activeTraceStepKey !== turnStepKey(turn.turnId, entry.id)"
                              class="mt-0.5 break-words text-[10px] leading-[1.2] text-stone-500 [overflow-wrap:anywhere]"
                            >
                              {{ timelinePreviewText(turn, entry) }}
                            </div>
                          </div>
                        </div>

                        <div class="flex items-center gap-1">
                          <span v-if="timelineDurationText(turn, entry)" class="shrink-0 whitespace-nowrap text-[10px] leading-[1.3] text-stone-400">
                            {{ timelineDurationText(turn, entry) }}
                          </span>
                          <button
                            class="inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                            type="button"
                            @click.stop="copyText(traceCopyKey(turn.turnId, entry.id), buildTimelineCopyText(turn, entry))"
                          >
                            <component :is="copiedKey === traceCopyKey(turn.turnId, entry.id) ? Check : Copy" class="h-3 w-3" />
                          </button>
                          <ChevronRight
                            class="h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                            :class="{ 'rotate-90': activeTraceStepKey === turnStepKey(turn.turnId, entry.id) }"
                          />
                        </div>
                      </button>

                      <div class="collapsible-body">
                        <div class="collapsible-content mt-1 pl-4">
                          <section>
                            <div class="space-y-1">
                              <div
                                v-for="row in buildTimelineRows(turn, entry)"
                                :key="turn.turnId + '-' + entry.id + '-' + row.label"
                                class="overflow-x-auto text-[10px] leading-[1.35]"
                              >
                                <div class="flex min-w-0 items-start gap-2">
                                  <span class="inline-flex shrink-0 items-center gap-1 whitespace-nowrap text-stone-400">
                                    <span>{{ row.label }}</span>
                                    <span>:</span>
                                  </span>
                                  <component
                                    v-if="row.inputKind"
                                    :is="inputKindIcon(row.inputKind)"
                                    class="h-3 w-3 shrink-0 text-stone-400"
                                  />
                                  <div
                                    class="min-w-0"
                                    :class="[rowToneClass(row.tone), row.multiline ? 'whitespace-pre-wrap text-left' : 'whitespace-nowrap text-left']"
                                  >
                                    <template v-if="row.expandable">
                                      {{ isExpandedResult(expandedResultKey(turn.turnId, entry.id, row.label)) ? row.value : previewResult(row.value) }}
                                      <button
                                        v-if="row.value.length > 240"
                                        class="ml-2 inline-flex text-[10px] text-stone-400 transition hover:text-stone-700"
                                        type="button"
                                        @click.stop="toggleExpandedResult(expandedResultKey(turn.turnId, entry.id, row.label))"
                                      >
                                        {{ isExpandedResult(expandedResultKey(turn.turnId, entry.id, row.label)) ? "收起" : "显示全部" }}
                                      </button>
                                    </template>
                                    <template v-else>
                                      {{ row.value }}
                                    </template>
                                  </div>
                                </div>
                              </div>
                            </div>
                          </section>

                          <template v-for="section in buildTimelineDetailSections(turn, entry)" :key="traceDetailKey(turn.turnId, entry.id, section.id)">
                            <section
                              v-if="section.kind === 'model'"
                              class="mt-2 border-l border-stone-200/80 pl-2"
                            >
                              <div class="flex items-start justify-between gap-1.5 py-0.5">
                                <div class="min-w-0 text-[10px] uppercase tracking-[0.14em] text-stone-400">
                                  {{ section.label }}
                                </div>
                                <button
                                  class="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                                  type="button"
                                  :data-testid="`trace-detail-button-${entry.id}-${section.id}`"
                                  @click.stop="copyText(traceDetailKey(turn.turnId, entry.id, section.id), section.content)"
                                >
                                  <component :is="copiedKey === traceDetailKey(turn.turnId, entry.id, section.id) ? Check : Copy" class="h-3 w-3" />
                                </button>
                              </div>
                              <div
                                class="mt-1 min-w-0 whitespace-pre-wrap break-words pl-4 text-[10px] leading-[1.25] [overflow-wrap:anywhere]"
                                :class="rowToneClass(section.tone)"
                              >
                                {{ section.content }}
                              </div>
                            </section>

                            <section
                              v-else
                              class="collapsible-shell overflow-hidden"
                              :class="section.kind === 'tool' ? 'mt-0.5' : 'mt-2'"
                              :data-open="activeTraceDetailKey === traceDetailKey(turn.turnId, entry.id, section.id)"
                            >
                              <button
                                class="flex w-full items-start justify-between gap-1 py-0 text-left"
                                type="button"
                                :data-testid="`trace-detail-button-${entry.id}-${section.id}`"
                                @click="toggleTraceDetail(turn.turnId, entry.id, section.id)"
                              >
                                <template v-if="section.kind === 'tool'">
                                  <div class="flex min-w-0 flex-1 items-center gap-1">
                                    <component
                                      :is="toolStatusIcon(section.toolStatus ?? 'planned')"
                                      class="h-3 w-3 shrink-0"
                                      :class="toolStatusIconClass(section.toolStatus ?? 'planned')"
                                    />
                                    <div class="min-w-0 truncate text-[10px] text-stone-500">
                                      {{ section.label }}
                                    </div>
                                  </div>
                                  <div class="ml-auto flex items-center gap-1">
                                    <span v-if="section.durationText" class="shrink-0 whitespace-nowrap text-[10px] text-stone-400">
                                      {{ section.durationText }}
                                    </span>
                                    <button
                                      class="inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                                      type="button"
                                      @click.stop="copyText(traceDetailKey(turn.turnId, entry.id, section.id), section.content)"
                                    >
                                      <component :is="copiedKey === traceDetailKey(turn.turnId, entry.id, section.id) ? Check : Copy" class="h-3 w-3" />
                                    </button>
                                    <ChevronRight
                                      class="mt-0.5 h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                                      :class="{ 'rotate-90': activeTraceDetailKey === traceDetailKey(turn.turnId, entry.id, section.id) }"
                                    />
                                  </div>
                                </template>
                                <template v-else>
                                  <div class="min-w-0">
                                    <div class="text-[10px] uppercase tracking-[0.14em] text-stone-400">
                                      {{ section.label }}
                                    </div>
                                    <div
                                      v-if="section.summary && activeTraceDetailKey !== traceDetailKey(turn.turnId, entry.id, section.id)"
                                      class="mt-0.5 pl-1 break-words text-[10px] leading-[1.2] text-stone-500 [overflow-wrap:anywhere]"
                                    >
                                      {{ section.summary }}
                                    </div>
                                  </div>
                                  <ChevronRight
                                    class="mt-0.5 h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                                    :class="{ 'rotate-90': activeTraceDetailKey === traceDetailKey(turn.turnId, entry.id, section.id) }"
                                  />
                                </template>
                              </button>

                              <div class="collapsible-body">
                                <div class="collapsible-content mt-0">
                                  <div
                                    class="min-w-0 whitespace-pre-wrap break-words text-[10px] leading-[1.25] [overflow-wrap:anywhere]"
                                    :class="rowToneClass(section.tone)"
                                  >
                                    {{ section.content }}
                                  </div>
                                </div>
                              </div>
                            </section>
                          </template>
                        </div>
                      </div>
                    </section>
                  </div>
                </div>
              </section>
            </section>
          </div>
        </section>
      </div>
    </ScrollArea>
  </aside>
</template>

<style scoped>
.collapsible-shell > .collapsible-body {
  display: grid;
  grid-template-rows: 0fr;
  min-height: 0;
  opacity: 0;
  overflow: hidden;
  transition:
    grid-template-rows 260ms cubic-bezier(0.2, 0.72, 0.18, 1),
    opacity 180ms ease;
}

.collapsible-shell[data-open="true"] > .collapsible-body {
  grid-template-rows: 1fr;
  min-height: 0;
  opacity: 1;
}

.collapsible-content {
  min-height: 0;
  overflow: hidden;
}
</style>
