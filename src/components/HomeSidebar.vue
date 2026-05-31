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
import type { BuildContextObservation, ToolActivity, TraceStep, TurnTraceRecord } from "@/types/runtime";
import { useRuntimeStore } from "@/stores/runtime";
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

const runtimeStore = useRuntimeStore();

const {
  availableTools,
  error,
  fallbackReason,
  firstTokenLatencyMs,
  messages,
  phaseLabel,
  providerMode,
  providerModel,
  providerName,
  providerProtocol,
  retrievedContext,
  sessionId,
  sessionSummary,
  turnTraceHistory
} = storeToRefs(runtimeStore);

const activePanel = ref<"status" | "trace" | "tools" | "">("trace");
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
const retrievedRecentHistoryCount = computed(() => retrievedSessionContext.value?.recentHistory?.length ?? 0);
const retrievedRecentAttachmentCount = computed(
  () => retrievedSessionContext.value?.recentAttachmentAssets?.length ?? 0
);
const retrievedLastReferencedFile = computed(() => retrievedSessionContext.value?.lastReferencedFile?.trim() ?? "");
const retrievedRunGoal = computed(() => retrievedRunState.value?.goal?.trim() ?? "");
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
const longTermMemoryStatusLabel = computed(() => {
  const status = retrievedLongTermMemory.value?.status?.trim().toLowerCase() ?? "";
  if (status === "available") {
    return `${longTermMemoryEntries.value.length} 条`;
  }
  if (status === "empty") {
    return "空";
  }

  return status || "";
});

const orderedTurnTraces = computed(() => [...turnTraceHistory.value]);
const latestTurnId = computed(() => orderedTurnTraces.value[orderedTurnTraces.value.length - 1]?.turnId ?? "");

function hasText(value?: string | null) {
  return Boolean(value?.trim());
}

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

function detectInputKind(content: string): InputKind {
  const normalized = content.trim().toLowerCase();

  if (
    normalized.includes("data:image") ||
    normalized.includes("![") ||
    /\.(png|jpe?g|gif|webp|bmp|svg)(\?.*)?$/.test(normalized)
  ) {
    return "image";
  }

  if (/\.(mp4|mov|avi|webm|mkv)(\?.*)?$/.test(normalized)) {
    return "video";
  }

  if (/\.(mp3|wav|m4a|aac|ogg|flac)(\?.*)?$/.test(normalized)) {
    return "audio";
  }

  return "text";
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

function providerModelLabel(turn: TurnTraceRecord) {
  const provider = turn.providerName?.trim();
  const model = turn.providerModel?.trim();

  if (provider && model) {
    return `${provider}/${model}`;
  }

  return model || provider || "";
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

  return metrics;
}

function turnDurationText(turn: TurnTraceRecord) {
  return turn.turnDurationMs != null ? formatDurationMs(turn.turnDurationMs) : "";
}

function stepDurationText(turn: TurnTraceRecord, step: TraceStep) {
  if (step.id === "step-call-model" && turn.firstTokenLatencyMs != null) {
    return `延时 ${turn.firstTokenLatencyMs} ms`;
  }

  if (step.id === "step-call-tool" && turn.toolActivities.length) {
    const totalSeconds = turn.toolActivities.reduce((sum, tool) => sum + (tool.durationSeconds ?? 0), 0);
    return totalSeconds > 0 ? formatDuration(totalSeconds) : "";
  }

  return "";
}

function stepTokenStats(turn: TurnTraceRecord, step: TraceStep) {
  if (step.id !== "step-call-model") {
    return [];
  }

  return buildTokenMetrics(turn);
}

function stepPreviewText(turn: TurnTraceRecord, step: TraceStep) {
  if (step.id !== "step-return") {
    return "";
  }

  const assistantMessage = turnAssistantMessage(turn.turnId);
  if (!assistantMessage || assistantMessage.status === "pending" || !hasText(assistantMessage.content)) {
    return "";
  }

  return previewInline(assistantMessage.content.trim(), 120);
}

function displayStepLabel(step: TraceStep) {
  return step.label.toUpperCase();
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

function buildPendingStepText(step: TraceStep) {
  if (step.id === "step-call-tool") {
    return "本轮未触发工具调用。";
  }

  if (step.id === "step-call-model") {
    return "尚未开始等待模型返回。";
  }

  return "该步骤未进行。";
}

function buildStepErrorText(turn: TurnTraceRecord, step: TraceStep) {
  if (step.state !== "error") {
    return "";
  }

  if (step.id === "step-call-tool") {
    const erroredTools = turn.toolActivities.filter((tool) => tool.status === "error");
    if (erroredTools.length) {
      return erroredTools
        .map((tool) => {
          const parts = [tool.name];
          if (tool.resultText?.trim()) {
            parts.push(tool.resultText.trim());
          } else if (tool.summary.trim()) {
            parts.push(tool.summary.trim());
          }

          return parts.join(": ");
        })
        .join("\n\n");
    }
  }

  return turn.error?.trim() ?? "";
}

function stepPurposeText(step: TraceStep) {
  switch (step.id) {
    case "step-plan":
      return "确认本轮到底收到了什么输入，以及是否带着图片或附件语义进入后续链路。";
    case "step-context":
      return "确认真正进入请求的内容，而不是只看 retrieval 快照，避免把上下文状态和实际请求混为一谈。";
    case "step-call-model":
      return "确认调用了哪一个 provider / model，以及模型何时开始产生首个可见增量。";
    case "step-call-tool":
      return "确认是否触发工具、用了什么参数、结果是否成功，以及工具调用的累计开销。";
    case "step-return":
      return "确认最终回给用户的内容、摘要和异常信号，判断这一轮是否真正完成。";
    default:
      return "";
  }
}

function turnCopyKey(turnId: string) {
  return `turn:${turnId}`;
}

function buildStepDetailSections(turn: TurnTraceRecord, step: TraceStep) {
  const userMessage = turnUserMessage(turn.turnId);
  const assistantMessage = turnAssistantMessage(turn.turnId);
  const sections: TraceDetailSection[] = [];
  const buildContextObservation = turn.buildContextObservation;
  const shouldRenderFinalReturn = assistantMessage?.status && assistantMessage.status !== "pending";

  if (step.id === "step-plan" && hasText(userMessage?.content)) {
    sections.push({
      id: "input-message",
      label: "输入原文",
      summary: previewInline(userMessage!.content.trim()),
      content: userMessage!.content.trim()
    });
  }

  if (step.id === "step-context" && buildContextObservation) {
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
        summary: previewInline(content),
        content
      });
    }
  }

  if (step.id === "step-call-model" && hasText(turn.fallbackReason)) {
    sections.push({
      id: "fallback",
      label: "降级原因",
      summary: previewInline(turn.fallbackReason!.trim()),
      content: turn.fallbackReason!.trim(),
      tone: "warning"
    });
  }

  if (step.id === "step-call-tool" && turn.toolActivities.length) {
    turn.toolActivities.forEach((tool, index) => {
      const lines = [];

      if (tool.argumentsText?.trim()) {
        lines.push(`参数:\n${tool.argumentsText.trim()}`);
      }

      if (tool.resultText?.trim()) {
        lines.push(`${tool.status === "error" ? "错误" : "结果"}:\n${tool.resultText.trim()}`);
      }

      if (tool.durationSeconds != null) {
        lines.push(`耗时: ${formatDuration(tool.durationSeconds)}`);
      }

      sections.push({
        id: `tool-${index + 1}`,
        label: tool.name,
        content: lines.join("\n\n"),
        tone: tool.status === "error" ? "danger" : "default",
        kind: "tool",
        toolStatus: tool.status,
        durationText: formatDuration(tool.durationSeconds)
      });
    });
  }

  if (step.id === "step-return" && shouldRenderFinalReturn) {
    if (hasText(assistantMessage?.reasoningContent)) {
      sections.push({
        id: "reasoning",
        label: "思考链",
        summary: previewInline(assistantMessage!.reasoningContent!.trim()),
        content: assistantMessage!.reasoningContent!.trim()
      });
    }

    if (hasText(assistantMessage?.content)) {
      sections.push({
        id: "assistant-output",
        label: "最终回复",
        summary: previewInline(assistantMessage!.content.trim()),
        content: assistantMessage!.content.trim()
      });
    }
  }

  return sections;
}

function buildStepRows(turn: TurnTraceRecord, step: TraceStep) {
  const userMessage = turnUserMessage(turn.turnId);
  const rows: DetailRow[] = [];

  if (step.id === "step-plan") {
    pushRow(rows, "输入类型", hasText(userMessage?.content) ? detectInputKind(userMessage!.content) : null);
    pushRow(rows, "图片输入", turn.buildContextObservation?.imageCount ?? null);
  }

  if (step.id === "step-context") {
    buildContextRows(turn.buildContextObservation).forEach((row) => rows.push(row));
    pushRow(rows, "请求目标", turn.providerRequestedName);
    pushRow(rows, "实际 provider", turn.providerName);
    pushRow(rows, "Protocol", turn.providerProtocol);
    pushRow(rows, "Model", turn.providerModel);
    pushRow(rows, "来源", turn.providerSource);
    pushRow(rows, "模式", turn.providerMode);
    pushRow(rows, "观测说明", turn.buildContextObservation ? "这里展示的是本轮真正发给模型的请求，不是 retrieval state 的替身。" : "当前还没有可展示的 request observation。", {
      multiline: true,
      tone: "muted"
    });
  }

  if (step.id === "step-call-model") {
    pushRow(rows, "模型", providerModelLabel(turn));
    pushRow(rows, "来源", turn.providerSource);
    pushRow(rows, "协议", turn.providerProtocol);
    pushRow(rows, "模式", turn.providerMode);
    pushRow(rows, "输入", turn.inputTokens);
    pushRow(rows, "命中缓存", cacheHitInputTokens(turn));
    pushRow(rows, "思考链", reasoningTokens(turn));
    pushRow(rows, "输出", turn.outputTokens);
    pushRow(rows, "延时", turn.firstTokenLatencyMs != null ? `${turn.firstTokenLatencyMs} ms` : null);
    pushRow(
      rows,
      "观测口径",
      turn.firstTokenLatencyMs != null
        ? "按首次收到 provider 增量计算，不等于整轮完成耗时。"
        : "当前路径没有真实流式首包事件，所以不展示首个增量统计。",
      { multiline: true, tone: "muted" }
    );
    pushRow(rows, "状态", step.state === "pending" ? buildPendingStepText(step) : null, { tone: "muted" });
    pushRow(rows, "错误", buildStepErrorText(turn, step), { multiline: true, tone: "danger" });
  }

  if (step.id === "step-call-tool") {
    pushRow(rows, "调用次数", turn.toolActivities.length);
    pushRow(rows, "失败次数", turn.toolActivities.filter((tool) => tool.status === "error").length);
    pushRow(rows, "累计耗时", stepDurationText(turn, step));
    pushRow(rows, "状态", step.state === "pending" ? buildPendingStepText(step) : null, { tone: "muted" });
    pushRow(rows, "错误", buildStepErrorText(turn, step), { multiline: true, tone: "danger" });
  }

  if (step.id === "step-return") {
    pushRow(rows, "状态", step.state === "pending" ? buildPendingStepText(step) : null, { tone: "muted" });
    pushRow(rows, "错误", buildStepErrorText(turn, step), { multiline: true, tone: "danger" });
  }

  return rows;
}

function buildCopyText(turn: TurnTraceRecord, step: TraceStep) {
  return [
    `${turn.title} / ${displayStepLabel(step)}`,
    ...buildStepRows(turn, step).map((row) => `${row.label}: ${row.value}`),
    ...buildStepDetailSections(turn, step).map((section) => `${section.label}:\n${section.content}`)
  ].join("\n\n");
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

  turn.traceSteps.forEach((step) => {
    parts.push(buildCopyText(turn, step));
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

function togglePanel(panel: "status" | "trace" | "tools") {
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
        <section class="collapsible-shell border-b border-stone-200/70 pb-4" :data-open="activePanel === 'status'">
          <button class="flex w-full items-center justify-between gap-3 text-left" type="button" data-testid="status-panel-toggle" @click="togglePanel('status')">
            <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
              <ScanSearch class="h-3.5 w-3.5" />
              <span>状态</span>
            </div>
            <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': activePanel === 'status' }" />
          </button>

          <div class="collapsible-body">
            <section class="collapsible-content mt-3 space-y-3">
              <div class="grid gap-2 text-[13px] leading-5 text-stone-600">
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
                <div v-if="firstTokenLatencyMs != null" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">延时</span>
                  <span class="text-right text-stone-800">{{ firstTokenLatencyMs }} ms</span>
                </div>
              </div>

              <p v-if="sessionSummary" class="break-words text-[12px] leading-5 text-stone-600 [overflow-wrap:anywhere]">
                {{ sessionSummary }}
              </p>
              <p v-if="fallbackReason" class="break-words text-[12px] leading-5 text-amber-800 [overflow-wrap:anywhere]">
                {{ fallbackReason }}
              </p>
              <p v-if="error" class="break-words text-[12px] leading-5 text-rose-700 [overflow-wrap:anywhere]">
                {{ error }}
              </p>
            </section>
          </div>
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
              <div
                v-if="retrievedSessionContext || retrievedRunState || retrievedLongTermMemory"
                class="mb-2 rounded-[0.5rem] border border-stone-200/80 bg-stone-50/75 px-3 py-2"
                data-testid="retrieved-context-summary"
              >
                <div class="flex items-center justify-between gap-3 text-[10px] uppercase tracking-[0.16em] text-stone-500">
                  <span>Current retrieval state</span>
                  <span v-if="retrievedSessionContext?.conversationId" class="truncate text-right">
                    {{ retrievedSessionContext?.conversationId }}
                  </span>
                </div>
                <div class="mt-2 grid gap-2 text-[12px] leading-5 text-stone-600">
                  <div class="flex items-start justify-between gap-3">
                    <span class="text-stone-400">Recent history</span>
                    <span class="text-right text-stone-800">{{ retrievedRecentHistoryCount }}</span>
                  </div>
                  <div class="flex items-start justify-between gap-3">
                    <span class="text-stone-400">Recent attachments</span>
                    <span class="text-right text-stone-800">{{ retrievedRecentAttachmentCount }}</span>
                  </div>
                  <div v-if="longTermMemoryStatusLabel" class="flex items-start justify-between gap-3">
                    <span class="text-stone-400">Long-term memory</span>
                    <span class="text-right text-stone-800">{{ longTermMemoryStatusLabel }}</span>
                  </div>
                  <div v-if="retrievedRunPhase" class="flex items-start justify-between gap-3">
                    <span class="text-stone-400">Run phase</span>
                    <span class="text-right text-stone-800">{{ retrievedRunPhase }}</span>
                  </div>
                </div>
                <p
                  v-if="retrievedRunGoal"
                  class="mt-2 break-words text-[11px] leading-5 text-stone-600 [overflow-wrap:anywhere]"
                  data-testid="retrieved-run-goal"
                >
                  Goal: {{ retrievedRunGoal }}
                </p>
                <p
                  v-if="retrievedActiveTaskFocus"
                  class="mt-2 break-words text-[11px] leading-5 text-stone-600 [overflow-wrap:anywhere]"
                  data-testid="retrieved-active-task"
                >
                  Active task: {{ retrievedActiveTaskFocus }}
                </p>
                <p
                  v-if="retrievedLastReferencedFile"
                  class="mt-2 break-words text-[11px] leading-5 text-stone-600 [overflow-wrap:anywhere]"
                  data-testid="retrieved-last-file"
                >
                  Last file: {{ retrievedLastReferencedFile }}
                </p>
                <div
                  v-if="longTermMemoryPreviewEntries.length"
                  class="mt-2 space-y-1 border-t border-stone-200/80 pt-2"
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
              </div>

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
                      v-for="step in turn.traceSteps"
                      :key="step.id"
                      class="collapsible-shell overflow-hidden py-0.5"
                      :data-open="activeTraceStepKey === turnStepKey(turn.turnId, step.id)"
                    >
                      <button
                        class="flex w-full items-start justify-between gap-1.5 text-left"
                        type="button"
                        :data-testid="`trace-step-button-${step.id}`"
                        @click="toggleTraceStep(turn.turnId, step.id)"
                      >
                        <div class="min-w-0 space-y-0.5">
                          <div class="flex min-w-0 items-center gap-1.5 text-[11px] leading-[1.3] text-stone-700">
                            <component
                              :is="traceStateIcon(step.state)"
                              class="h-3 w-3 shrink-0"
                              :class="{
                                'text-stone-500': step.state === 'completed',
                                'animate-spin text-stone-500': step.state === 'active',
                                'text-rose-600': step.state === 'error',
                                'text-stone-300': step.state === 'pending'
                              }"
                            />
                            <span class="truncate">{{ displayStepLabel(step) }}</span>
                          </div>
                          <div class="pl-[1.125rem] text-[10px] text-stone-400">
                            <div class="flex flex-wrap items-center gap-2">
                            <span
                              v-for="stat in stepTokenStats(turn, step)"
                                :key="turn.turnId + '-' + step.id + '-' + stat"
                              class="text-stone-400"
                            >
                              {{ stat }}
                            </span>
                            <span v-if="stepDurationText(turn, step)" class="text-stone-400">
                              {{ stepDurationText(turn, step) }}
                            </span>
                            </div>
                            <div
                              v-if="stepPreviewText(turn, step) && activeTraceStepKey !== turnStepKey(turn.turnId, step.id)"
                              class="mt-0.5 break-words text-[10px] leading-[1.35] text-stone-500 [overflow-wrap:anywhere]"
                            >
                              {{ stepPreviewText(turn, step) }}
                            </div>
                          </div>
                        </div>

                        <div class="flex items-center gap-1">
                          <button
                            class="inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                            type="button"
                            @click.stop="copyText(traceCopyKey(turn.turnId, step.id), buildCopyText(turn, step))"
                          >
                            <component :is="copiedKey === traceCopyKey(turn.turnId, step.id) ? Check : Copy" class="h-3 w-3" />
                          </button>
                          <ChevronRight
                            class="h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                            :class="{ 'rotate-90': activeTraceStepKey === turnStepKey(turn.turnId, step.id) }"
                          />
                        </div>
                      </button>

                      <div class="collapsible-body">
                        <div class="collapsible-content mt-1 pl-4">
                          <div class="mb-2 flex items-start gap-1.5 text-[10px] font-light leading-[1.35] text-stone-400 [overflow-wrap:anywhere]">
                            <Info class="mt-[1px] h-3 w-3 shrink-0" />
                            <p class="min-w-0 break-words [overflow-wrap:anywhere]">
                              {{ stepPurposeText(step) }}
                            </p>
                          </div>
                          <section>
                            <div class="space-y-1">
                              <div
                                v-for="row in buildStepRows(turn, step)"
                                :key="turn.turnId + '-' + step.id + '-' + row.label"
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
                                      {{ isExpandedResult(expandedResultKey(turn.turnId, step.id, row.label)) ? row.value : previewResult(row.value) }}
                                      <button
                                        v-if="row.value.length > 240"
                                        class="ml-2 inline-flex text-[10px] text-stone-400 transition hover:text-stone-700"
                                        type="button"
                                        @click.stop="toggleExpandedResult(expandedResultKey(turn.turnId, step.id, row.label))"
                                      >
                                        {{ isExpandedResult(expandedResultKey(turn.turnId, step.id, row.label)) ? "收起" : "显示全部" }}
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
                            v-for="section in buildStepDetailSections(turn, step)"
                            :key="traceDetailKey(turn.turnId, step.id, section.id)"
                            class="collapsible-shell mt-2 overflow-hidden border-l border-stone-200/80 pl-2"
                            :data-open="activeTraceDetailKey === traceDetailKey(turn.turnId, step.id, section.id)"
                          >
                            <button
                              class="flex w-full items-start justify-between gap-1.5 py-0.5 text-left"
                              type="button"
                              :data-testid="`trace-detail-button-${step.id}-${section.id}`"
                              @click="toggleTraceDetail(turn.turnId, step.id, section.id)"
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
                                    @click.stop="copyText(traceDetailKey(turn.turnId, step.id, section.id), section.content)"
                                  >
                                    <component :is="copiedKey === traceDetailKey(turn.turnId, step.id, section.id) ? Check : Copy" class="h-3 w-3" />
                                  </button>
                                  <ChevronRight
                                    class="mt-0.5 h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                                    :class="{ 'rotate-90': activeTraceDetailKey === traceDetailKey(turn.turnId, step.id, section.id) }"
                                  />
                                </div>
                              </template>
                              <template v-else>
                                <div class="min-w-0">
                                  <div class="text-[10px] uppercase tracking-[0.14em] text-stone-400">
                                    {{ section.label }}
                                  </div>
                                  <div
                                    v-if="section.summary && activeTraceDetailKey !== traceDetailKey(turn.turnId, step.id, section.id)"
                                    class="mt-0.5 pl-1 break-words text-[10px] leading-[1.35] text-stone-500 [overflow-wrap:anywhere]"
                                  >
                                    {{ section.summary }}
                                  </div>
                                </div>
                                <ChevronRight
                                  class="mt-0.5 h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                                  :class="{ 'rotate-90': activeTraceDetailKey === traceDetailKey(turn.turnId, step.id, section.id) }"
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
