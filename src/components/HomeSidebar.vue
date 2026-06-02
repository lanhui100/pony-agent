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
  Info,
  LoaderCircle,
  Orbit,
  ScanSearch,
  Video,
  Wrench
} from "lucide-vue-next";
import { extractActiveTaskFocus } from "@/types/runtime";
import type { BuildContextObservation, ToolActivity, TraceStep, TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";
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
  kind?: "default" | "tool";
  toolStatus?: ToolActivity["status"];
  durationText?: string;
};

type TimelineDetailRow = DetailRow;

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();

const {
  availableTools,
  error,
  fallbackReason,
  messages,
  phaseLabel,
  providerMode,
  providerModel,
  providerName,
  providerProtocol,
  retrievedContext,
  sessionId,
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

const orderedTurnTraces = computed(() => [...turnTraceHistory.value]);
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
          label: "CALL MODEL #1"
        });
        continue;
      }

      const modelIndex = normalized.length - 1 - reverseModelIndex;
      const modelEntry = normalized[modelIndex];
      normalized[modelIndex] = {
        ...modelEntry,
        kind: "call_model",
        state: entry.state ?? modelEntry.state,
        text: entry.text ?? modelEntry.text ?? null,
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
  const metrics = buildTokenMetrics(turn);

  if (turn.firstTokenLatencyMs != null) {
    metrics.push(`延时 ${turn.firstTokenLatencyMs} ms`);
  }

  return metrics.join(" · ");
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

function turnUserMessage(turnId: string) {
  return messages.value.find((message) => message.turnId === turnId && message.role === "user") ?? null;
}

function turnAssistantMessage(turnId: string) {
  return messages.value.find((message) => message.turnId === turnId && message.role === "assistant") ?? null;
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

function formatCompactDurationMs(durationMs?: number | null) {
  if (durationMs == null) {
    return "";
  }

  return durationMs < 1000 ? `${Math.round(durationMs)} ms` : `${(durationMs / 1000).toFixed(1)} s`;
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

  const activeGenerationMs = turn.firstTokenLatencyMs != null
    ? Math.max(turn.turnDurationMs - turn.firstTokenLatencyMs, 1)
    : Math.max(turn.turnDurationMs, 1);

  const tokensPerSecond = turn.outputTokens / (activeGenerationMs / 1000);
  return Number.isFinite(tokensPerSecond) ? tokensPerSecond : null;
}

function formatTokenSpeed(turn: TurnTraceRecord) {
  const value = tokenSpeed(turn);
  return value != null ? `${value.toFixed(1)} token/s` : "";
}

function formatEntryTokenSpeed(entry: TraceTimelineEntry) {
  if (entry.outputTokens == null || entry.turnDurationMs == null) {
    return "";
  }

  const activeGenerationMs = entry.firstTokenLatencyMs != null
    ? Math.max(entry.turnDurationMs - entry.firstTokenLatencyMs, 1)
    : Math.max(entry.turnDurationMs, 1);
  const value = entry.outputTokens / (activeGenerationMs / 1000);
  return Number.isFinite(value) ? `${value.toFixed(1)} token/s` : "";
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

function timelineMetricEntry(turn: TurnTraceRecord, entry: TraceTimelineEntry): TraceTimelineEntry {
  const record = timelineProviderCallRecord(turn, entry);
  const callModelEntries = turnTimeline(turn).filter((candidate) => canonicalTraceTimelineKind(candidate.kind) === "call_model");
  const isFinalCallModel = callModelEntries[callModelEntries.length - 1]?.id === entry.id;

  return {
    ...entry,
    inputTokens: entry.inputTokens ?? record?.inputTokens ?? (isFinalCallModel ? turn.inputTokens ?? null : null),
    cacheHitInputTokens:
      entry.cacheHitInputTokens ?? record?.cacheHitInputTokens ?? (isFinalCallModel ? cacheHitInputTokens(turn) ?? null : null),
    reasoningTokens:
      entry.reasoningTokens ?? record?.reasoningTokens ?? (isFinalCallModel ? reasoningTokens(turn) ?? null : null),
    outputTokens: entry.outputTokens ?? record?.outputTokens ?? (isFinalCallModel ? turn.outputTokens ?? null : null),
    totalTokens: entry.totalTokens ?? record?.totalTokens ?? (isFinalCallModel ? turn.totalTokens ?? null : null),
    firstTokenLatencyMs:
      entry.firstTokenLatencyMs ?? record?.firstTokenLatencyMs ?? (isFinalCallModel ? turn.firstTokenLatencyMs ?? null : null),
    turnDurationMs: entry.turnDurationMs ?? (isFinalCallModel ? turn.turnDurationMs ?? null : null)
  };
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
    stats.push(`输入 ${metricEntry.inputTokens}`);
  }
  if (metricEntry.cacheHitInputTokens != null) {
    stats.push(`命中缓存 ${metricEntry.cacheHitInputTokens}`);
  }
  if (metricEntry.reasoningTokens != null) {
    stats.push(`思考链 ${metricEntry.reasoningTokens}`);
  }
  if (metricEntry.outputTokens != null) {
    stats.push(`输出 ${metricEntry.outputTokens}`);
  }
  const tokenSpeed = formatEntryTokenSpeed(metricEntry);
  if (tokenSpeed) {
    stats.push(tokenSpeed);
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
      const tokenSpeed = formatEntryTokenSpeed(metricEntry);
      const latencyText =
        metricEntry.firstTokenLatencyMs != null ? `延时 ${metricEntry.firstTokenLatencyMs} ms` : "";
      return [latencyText, metricEntry.turnDurationMs != null ? `耗时 ${formatDurationMs(metricEntry.turnDurationMs)}` : "", tokenSpeed]
        .filter(Boolean)
        .join(" · ");
    }
    if (metricEntry.firstTokenLatencyMs != null) {
      return `延时 ${metricEntry.firstTokenLatencyMs} ms`;
    }
  }

  return "";
}

function timelinePreviewText(entry: TraceTimelineEntry) {
  const kind = canonicalTraceTimelineKind(entry.kind);
  if (kind === "call_tool") {
    return entry.toolActivities?.[0]?.summary ?? "";
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

function timelinePurposeText(entry: TraceTimelineEntry) {
  switch (canonicalTraceTimelineKind(entry.kind)) {
    case "input":
      return "记录本轮进入 agent 的用户输入。";
    case "prepare_retrieval":
      return "记录 retrieval 参与请求准备的阶段信号。";
    case "build_context":
      return "记录本轮真正发送给模型前的上下文组织结果。";
    case "call_model":
      return "对应一次独立的模型调用，不与其他 hop 合并。";
    case "call_tool":
      return "对应一次独立的工具调用，不与其他 hop 合并。";
    default:
      return "";
  }
}

function buildTimelineRows(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const rows: TimelineDetailRow[] = [];
  const kind = canonicalTraceTimelineKind(entry.kind);

  if (kind === "input") {
    pushRow(rows, "标题", turn.title);
    pushRow(rows, "输入", timelinePreviewText(entry), { multiline: true });
    return rows;
  }

  if (kind === "prepare_retrieval") {
    pushRow(rows, "阶段", "prepare_retrieval");
    pushRow(
      rows,
      "观测说明",
      "这一阶段表示 retrieval 已参与本轮请求准备，后续 build_context 会展示真正发给模型的上下文摘要。",
      { multiline: true, tone: "muted" }
    );
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
    pushRow(rows, "Provider", entry.providerName);
    pushRow(rows, "Protocol", entry.providerProtocol);
    pushRow(rows, "Model", entry.providerModel);
    pushRow(rows, "来源", entry.providerSource);
    pushRow(rows, "模式", entry.providerMode);
    pushRow(rows, "输入", metricEntry.inputTokens);
    pushRow(rows, "命中缓存", metricEntry.cacheHitInputTokens);
    pushRow(rows, "思考链", metricEntry.reasoningTokens);
    pushRow(rows, "输出", metricEntry.outputTokens);
    pushRow(rows, "总计", metricEntry.totalTokens);
    pushRow(rows, "首 token", metricEntry.firstTokenLatencyMs != null ? `${metricEntry.firstTokenLatencyMs} ms` : null);
    pushRow(rows, "耗时", metricEntry.turnDurationMs != null ? formatDurationMs(metricEntry.turnDurationMs) : null);
    pushRow(rows, "速率", formatEntryTokenSpeed(metricEntry) || null);
    pushRow(rows, "输出预览", timelinePreviewText(entry), { multiline: true, expandable: true });
    pushRow(rows, "错误", entry.error, { multiline: true, tone: "danger" });
    return rows;
  }

  if (kind === "call_tool") {
    const parentTool = entry.toolActivities?.[0];
    pushRow(rows, "工具", parentTool?.name);
    pushRow(rows, "状态", parentTool?.status);
    pushRow(rows, "摘要", parentTool?.summary, { multiline: true });
    pushRow(rows, "耗时", timelineDurationText(turn, entry));
    pushRow(rows, "错误", entry.error, { multiline: true, tone: "danger" });
    return rows;
  }

  return rows;
}

function buildTimelineDetailSections(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const userMessage = turnUserMessage(turn.turnId);
  const assistantMessage = turnAssistantMessage(turn.turnId);
  const sections: TraceDetailSection[] = [];
  const kind = canonicalTraceTimelineKind(entry.kind);

  if (kind === "input") {
    const inputText = entry.text?.trim() || userMessage?.content?.trim() || "";
    if (inputText) {
      sections.push({
        id: "input-message",
        label: "输入原文",
        content: inputText,
        summary: previewInline(inputText)
      });
    }
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

  const reasoningContent = entry.reasoningContent?.trim() || assistantMessage?.reasoningContent?.trim() || "";
  if (reasoningContent) {
    sections.push({
      id: "reasoning",
      label: "思考链",
      content: reasoningContent,
      summary: previewResult(reasoningContent)
    });
  }

  const outputContent = entry.text?.trim() || assistantMessage?.content?.trim() || "";
  if (outputContent) {
    sections.push({
      id: "assistant-output",
      label: "模型输出",
      content: outputContent,
      summary: previewResult(outputContent)
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
  } else if (activity.summary.trim()) {
    lines.push(`摘要:\n${activity.summary.trim()}`);
  }

  if (activity.durationSeconds != null) {
    lines.push(`耗时: ${formatDuration(activity.durationSeconds)}`);
  }

  return lines.join("\n\n");
}

function buildTimelineCopyText(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const lines = [`${entry.label}`, `状态: ${entry.state}`];
  const preview = timelinePreviewText(entry);
  if (preview) {
    lines.push("", preview);
  }
  buildTimelineRows(turn, entry).forEach((row) => {
    lines.push(`${row.label}: ${row.value}`);
  });
  return lines.join("\n");
}

function buildTokenMetrics(turn: TurnTraceRecord) {
  const metrics: string[] = [];

  if (turn.inputTokens != null) {
    metrics.push(`输入 ${turn.inputTokens}`);
  }

  const cacheTokens = cacheHitInputTokens(turn);
  if (cacheTokens != null) {
    metrics.push(`命中缓存 ${cacheTokens}`);
  }

  const reasoning = reasoningTokens(turn);
  if (reasoning != null) {
    metrics.push(`思考链 ${reasoning}`);
  }

  if (turn.outputTokens != null) {
    metrics.push(`输出 ${turn.outputTokens}`);
  }

  const throughput = formatTokenSpeed(turn);
  if (throughput) {
    metrics.push(throughput);
  }

  return metrics;
}

function turnDurationText(turn: TurnTraceRecord) {
  return turn.turnDurationMs != null ? formatCompactDurationMs(turn.turnDurationMs) : "";
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

watch(
  orderedTurnTraces,
  (turns) => {
    if (!turns.some((turn) => turn.turnId === activeTurnId.value)) {
      activeTurnId.value = turns[turns.length - 1]?.turnId ?? "";
      activeTraceStepKey.value = "";
      activeTraceDetailKey.value = "";
    }
  },
  { deep: true }
);
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
                    <div v-if="turnMeta(turn)" class="pl-[1.125rem] text-[10px] leading-[1.3] text-stone-400">
                      {{ turnMeta(turn) }}
                    </div>
                  </div>
                  <div class="flex items-center gap-1">
                    <span v-if="turnDurationText(turn)" class="text-[10px] leading-[1.3] text-stone-400">
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
                    <p v-if="detailText(turn)" class="break-words text-[10px] leading-[1.35] text-stone-500 [overflow-wrap:anywhere]">
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
                          <div class="pl-[1.125rem] text-[10px] text-stone-400">
                            <div class="flex flex-wrap items-center gap-2">
                            <span
                              v-for="stat in timelineEntryStats(turn, entry)"
                                :key="turn.turnId + '-' + entry.id + '-' + stat"
                              class="text-stone-400"
                            >
                              {{ stat }}
                            </span>
                            <span v-if="timelineDurationText(turn, entry)" class="text-stone-400">
                              {{ timelineDurationText(turn, entry) }}
                            </span>
                            </div>
                            <div
                              v-if="timelinePreviewText(entry) && activeTraceStepKey !== turnStepKey(turn.turnId, entry.id)"
                              class="mt-0.5 break-words text-[10px] leading-[1.35] text-stone-500 [overflow-wrap:anywhere]"
                            >
                              {{ timelinePreviewText(entry) }}
                            </div>
                          </div>
                        </div>

                        <div class="flex items-center gap-1">
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
                          <div class="mb-2 flex items-start gap-1.5 text-[10px] font-light leading-[1.35] text-stone-400 [overflow-wrap:anywhere]">
                            <Info class="mt-[1px] h-3 w-3 shrink-0" />
                            <p class="min-w-0 break-words [overflow-wrap:anywhere]">
                              {{ timelinePurposeText(entry) }}
                            </p>
                          </div>
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

                          <section
                            v-for="section in buildTimelineDetailSections(turn, entry)"
                            :key="traceDetailKey(turn.turnId, entry.id, section.id)"
                            class="collapsible-shell mt-2 overflow-hidden border-l border-stone-200/80 pl-2"
                            :data-open="activeTraceDetailKey === traceDetailKey(turn.turnId, entry.id, section.id)"
                          >
                            <button
                              class="flex w-full items-start justify-between gap-1.5 py-0.5 text-left"
                              type="button"
                              :data-testid="`trace-detail-button-${entry.id}-${section.id}`"
                              @click="toggleTraceDetail(turn.turnId, entry.id, section.id)"
                            >
                              <template v-if="section.kind === 'tool'">
                                <div class="flex min-w-0 items-center gap-1.5">
                                  <component
                                    :is="toolStatusIcon(section.toolStatus ?? 'planned')"
                                    class="h-3 w-3 shrink-0"
                                    :class="toolStatusIconClass(section.toolStatus ?? 'planned')"
                                  />
                                  <div class="min-w-0 truncate text-[10px] uppercase tracking-[0.14em] text-stone-500">
                                    {{ section.label }}
                                  </div>
                                </div>
                                <div class="ml-auto flex items-center gap-1">
                                  <span v-if="section.durationText" class="text-[10px] text-stone-400">
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
                                    class="mt-0.5 pl-1 break-words text-[10px] leading-[1.35] text-stone-500 [overflow-wrap:anywhere]"
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
                              <div class="collapsible-content mt-1 pl-4">
                                <div
                                  class="min-w-0 whitespace-pre-wrap break-words text-[10px] leading-[1.4] [overflow-wrap:anywhere]"
                                  :class="rowToneClass(section.tone)"
                                >
                                  {{ section.content }}
                                </div>
                              </div>
                            </div>
                          </section>
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
