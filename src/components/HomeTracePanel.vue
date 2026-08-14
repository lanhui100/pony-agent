<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  AudioLines,
  Brain,
  Check,
  ChevronRight,
  Circle,
  Clock3,
  Copy,
  FileText,
  Gauge,
  Image as ImageIcon,
  LoaderCircle,
  Orbit,
  Video,
  Zap
} from "lucide-vue-next";
import type {
  BuildContextObservation,
  ProviderCallCacheRecord,
  ToolActivity,
  TraceStep,
  TraceTimelineEntry,
  TurnTraceRecord
} from "@/types/runtime";
import Tooltip from "@/components/ui/Tooltip.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import {
  computeVirtualTurnWindow,
  buildTurnPrefixHeights,
  estimateTurnHeight
} from "@/lib/runtime/trace-virtual-scroll";

type DetailRowTone = "default" | "muted" | "warning" | "danger";
type InputKind = "text" | "image" | "video" | "audio";

type DetailRow = {
  label: string;
  value: string;
  multiline?: boolean;
  tone?: DetailRowTone;
  inputKind?: InputKind;
  expandable?: boolean;
  icon?: object;
};

type MetricDisplayItem = {
  icon: object;
  tooltip: string;
  value: string;
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

const props = defineProps<{
  turns: TurnTraceRecord[];
  sessionId: string;
  copiedKey: string;
  open: boolean;
  canonicalKind: (kind: TraceTimelineEntry["kind"]) => TraceTimelineEntry["kind"];
  turnTimeline: (turn: TurnTraceRecord) => TraceTimelineEntry[];
  providerReturnedCacheHitInputTokens: (turn: TurnTraceRecord) => number | null;
}>();

const emit = defineEmits<{
  copy: [key: string, text: string];
  toggle: [];
}>();

const activeTurnId = ref("");
const activeTraceStepKey = ref("");
const activeTraceDetailKey = ref("");
const expandedResultKeys = ref<string[]>([]);

const latestTurnId = computed(() => props.turns[props.turns.length - 1]?.turnId ?? "");
// Signature only tracks turnId list — the watch below only needs to react
// to turn additions/removals, not to status/timeline updates within a turn.
const orderedTurnTraceSignature = computed(() =>
  props.turns.map((turn) => turn.turnId).join("|")
);
// Cached timeline per turn — computed once per turns prop change,
// shared by all consumers (template v-for, session stats, metric helpers).
const turnTimelineCache = computed(() => {
  const cache = new Map<string, TraceTimelineEntry[]>();
  for (const turn of props.turns) {
    cache.set(turn.turnId, props.turnTimeline(turn));
  }
  return cache;
});

// Reads from the pre-computed cache when available (during rendering),
// falls back to direct computation (for non-cached turns).
function getCachedTimeline(turn: TurnTraceRecord): TraceTimelineEntry[] {
  return turnTimelineCache.value.get(turn.turnId) ?? props.turnTimeline(turn);
}

// ===== turn 级虚拟滚动（PA-085）=====
// 独立滚动容器 ref（body 内嵌 ScrollArea），scrollTop/viewportHeight 驱动窗口。
const traceBodyScrollRef = ref<{ viewportEl: HTMLElement | null } | null>(null);
const traceScrollTop = ref(0);
const traceViewportHeight = ref(0);
let followBottom = true;
let scrollRafId: number | null = null;
let resizeObserver: ResizeObserver | null = null;

function turnHeightAt(turn: TurnTraceRecord): number {
  const timeline = getCachedTimeline(turn);
  const isTurnExpanded = activeTurnId.value === turn.turnId;
  const expandedEntryCount =
    activeTraceStepKey.value && activeTraceStepKey.value.startsWith(`${turn.turnId}:`)
      ? timeline.length
      : 0;
  return estimateTurnHeight(timeline, isTurnExpanded, expandedEntryCount);
}

const turnPrefixHeights = computed(() =>
  buildTurnPrefixHeights(props.turns, turnHeightAt)
);
const virtualTurnWindow = computed(() =>
  computeVirtualTurnWindow(turnPrefixHeights.value, traceScrollTop.value, traceViewportHeight.value)
);
const visibleTurns = computed(() =>
  props.turns.slice(virtualTurnWindow.value.startIndex, virtualTurnWindow.value.endIndex)
);
const virtualPaddingTop = computed(() => virtualTurnWindow.value.paddingTop);
const virtualPaddingBottom = computed(() => virtualTurnWindow.value.paddingBottom);

function handleTraceBodyScroll() {
  const viewportEl = traceBodyScrollRef.value?.viewportEl;
  if (!viewportEl) {
    return;
  }
  if (scrollRafId != null) {
    cancelAnimationFrame(scrollRafId);
  }
  scrollRafId = requestAnimationFrame(() => {
    scrollRafId = null;
    const scrollTop = viewportEl.scrollTop;
    traceScrollTop.value = scrollTop;
    // 跟随底部：视口接近内容底部时保持跟随；用户上滑后停止
    const distanceToBottom = viewportEl.scrollHeight - scrollTop - viewportEl.clientHeight;
    followBottom = distanceToBottom < 80;
  });
}

function scrollTraceToBottom(behavior: ScrollBehavior = "auto") {
  const viewportEl = traceBodyScrollRef.value?.viewportEl;
  if (!viewportEl) {
    return;
  }
  viewportEl.scrollTo({ top: viewportEl.scrollHeight, behavior });
  traceScrollTop.value = viewportEl.scrollTop;
}

// 挂载时测量视口高度并建立 ResizeObserver；
// scroll 监听直接绑定到 viewport 元素（自定义 ScrollArea 的 attrs 会落到 root，
// 原生 scroll 事件不冒泡，无法从组件上捕获）。
let boundScrollHandler: ((event: Event) => void) | null = null;

function onTraceBodyMounted() {
  const viewportEl = traceBodyScrollRef.value?.viewportEl;
  if (!viewportEl) {
    return;
  }
  traceViewportHeight.value = viewportEl.clientHeight;
  boundScrollHandler = handleTraceBodyScroll;
  viewportEl.addEventListener("scroll", boundScrollHandler, { passive: true });
  if (typeof ResizeObserver !== "undefined" && resizeObserver == null) {
    resizeObserver = new ResizeObserver(() => {
      traceViewportHeight.value = viewportEl.clientHeight;
    });
    resizeObserver.observe(viewportEl);
  }
}

// 首次展开时定位到最新 turn（底部）；流式更新时若处于跟随态则保持底部
watch(
  () => props.open,
  (isOpen) => {
    if (isOpen) {
      // 等 body 挂载完成后再定位
      requestAnimationFrame(() => {
        scrollTraceToBottom();
        onTraceBodyMounted();
      });
    }
  }
);

watch(
  orderedTurnTraceSignature,
  () => {
    if (!props.open) {
      return;
    }
    if (followBottom) {
      requestAnimationFrame(() => {
        scrollTraceToBottom();
      });
    }
  }
);

// 流式更新只改变当前 turn 的 timeline 内容（turn ID 不变），
// 因此底部跟随需依赖总高度变化而非 turn ID 签名。
const totalTraceHeight = computed(() =>
  turnPrefixHeights.value.length > 0
    ? turnPrefixHeights.value[turnPrefixHeights.value.length - 1]!
    : 0
);

watch(totalTraceHeight, () => {
  if (!props.open) {
    return;
  }
  if (followBottom) {
    requestAnimationFrame(() => {
      scrollTraceToBottom();
    });
  }
});

watch(
  () => props.sessionId,
  () => {
    followBottom = true;
    traceScrollTop.value = 0;
  }
);

function onTraceBodyUnmounted() {
  resizeObserver?.disconnect();
  resizeObserver = null;
  if (boundScrollHandler != null) {
    const viewportEl = traceBodyScrollRef.value?.viewportEl;
    viewportEl?.removeEventListener("scroll", boundScrollHandler);
    boundScrollHandler = null;
  }
  if (scrollRafId != null) {
    cancelAnimationFrame(scrollRafId);
    scrollRafId = null;
  }
}

function isCheckpointPersistEntry(entry: TraceTimelineEntry) {
  return entry.label === "PERSIST CHECKPOINT";
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

/** 解析最终请求消息文本，拆分为单条消息块 */
function parseRequestMessages(text: string): Array<{ index: number; header: string; content: string }> {
  const blocks: Array<{ index: number; header: string; content: string }> = [];
  // 消息格式: "[0] role\ncontent"，消息之间用 \n\n 分隔
  const pattern = /^\[(\d+)\]\s*(.+)$/m;
  let remaining = text.trim();
  while (remaining) {
    const match = remaining.match(pattern);
    if (!match) { break; }
    const headerLine = match[0];
    const idx = parseInt(match[1], 10);
    const role = match[2];
    const afterHeader = remaining.slice(headerLine.length).trimStart();
    // 找下一条消息的起始位置
    const nextMatch = afterHeader.match(/^\[\d+\]\s*.+$/m);
    let content: string;
    if (nextMatch) {
      content = afterHeader.slice(0, nextMatch.index!).trim();
      remaining = afterHeader.slice(nextMatch.index!);
    } else {
      content = afterHeader.trim();
      remaining = "";
    }
    blocks.push({ index: idx, header: `[${idx}] ${role}`, content });
  }
  return blocks;
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

  if (entry.firstTokenLatencyMs == null) {
    return Math.max(entry.turnDurationMs, 1);
  }

  const generationDurationMs = entry.turnDurationMs - entry.firstTokenLatencyMs;
  return generationDurationMs > 0 ? generationDurationMs : null;
}

function formatTokenSpeedLabel(entry: TraceTimelineEntry) {
  return entry.firstTokenLatencyMs == null ? "整体速度" : "生成速度";
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
  return Number.isFinite(value) ? `${value.toFixed(1)} t/s` : "";
}

function average(values: number[]) {
  if (!values.length) {
    return null;
  }

  const total = values.reduce((sum, value) => sum + value, 0);
  return total / values.length;
}

function timelineCallModelIndex(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  return getCachedTimeline(turn)
    .filter((candidate) => props.canonicalKind(candidate.kind) === "call_model")
    .findIndex((candidate) => candidate.id === entry.id);
}

function timelineProviderCallRecord(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  if (props.canonicalKind(entry.kind) !== "call_model") {
    return null;
  }

  const index = timelineCallModelIndex(turn, entry);
  return index >= 0 ? turn.providerCallRecords?.[index] ?? null : null;
}

function timelineMetricEntry(turn: TurnTraceRecord, entry: TraceTimelineEntry, options: { allowTurnFallback?: boolean } = {}): TraceTimelineEntry {
  const record = timelineProviderCallRecord(turn, entry);
  const allowTurnFallback = options.allowTurnFallback ?? true;
  const callModelEntries = getCachedTimeline(turn).filter((candidate) => props.canonicalKind(candidate.kind) === "call_model");
  const useTurnFallback = allowTurnFallback && callModelEntries.length === 1 && callModelEntries[0]?.id === entry.id;

  if (record) {
    return {
      ...entry,
      inputTokens: record.inputTokens ?? null,
      cacheHitInputTokens: record.cacheHitInputTokens ?? null,
      reasoningTokens: record.reasoningTokens ?? null,
      outputTokens: record.outputTokens ?? null,
      totalTokens: record.totalTokens ?? null,
      firstTokenLatencyMs: effectiveFirstTokenLatencyMs(
        {
          ...entry,
          firstTokenLatencyMs: record.firstTokenLatencyMs ?? null,
          turnDurationMs: record.turnDurationMs ?? null
        },
        record
      ),
      turnDurationMs: record.turnDurationMs ?? null
    };
  }

  return {
    ...entry,
    inputTokens: entry.inputTokens ?? (useTurnFallback ? turn.inputTokens ?? null : null),
    cacheHitInputTokens: null,
    reasoningTokens:
      entry.reasoningTokens ?? (useTurnFallback ? reasoningTokens(turn) ?? null : null),
    outputTokens: entry.outputTokens ?? (useTurnFallback ? turn.outputTokens ?? null : null),
    totalTokens: entry.totalTokens ?? (useTurnFallback ? turn.totalTokens ?? null : null),
    firstTokenLatencyMs: effectiveFirstTokenLatencyMs(
      {
        ...entry,
        firstTokenLatencyMs: entry.firstTokenLatencyMs ?? (useTurnFallback ? turn.firstTokenLatencyMs ?? null : null),
        turnDurationMs: entry.turnDurationMs ?? (useTurnFallback ? turn.turnDurationMs ?? null : null)
      },
      null
    ),
    turnDurationMs: entry.turnDurationMs ?? (useTurnFallback ? turn.turnDurationMs ?? null : null)
  };
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
  const callModelEntries = getCachedTimeline(turn).filter((entry) => props.canonicalKind(entry.kind) === "call_model");
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
  const cacheTotal = caches.length ? caches.reduce((sum, value) => sum + value, 0) : props.providerReturnedCacheHitInputTokens(turn);
  const outputTotal = outputs.length ? outputs.reduce((sum, value) => sum + value, 0) : turn.outputTokens ?? null;
  const speedAverage = outputTotal != null && generationDurations.length
    ? outputTotal / (generationDurations.reduce((sum, value) => sum + value, 0) / 1000)
    : tokenSpeed(turn);
  const speedLabel = latencies.length === generationDurations.length && generationDurations.length > 0
    ? "生成速度"
    : latencies.length === 0
      ? "整体速度"
      : "速度";
  const latencyAverage = latencies.length ? average(latencies) : turn.firstTokenLatencyMs ?? null;

  if (inputTotal != null) {
    metrics.push(`输入 ${formatInteger(inputTotal)}`);
  }
  if (outputTotal != null) {
    metrics.push(`输出 ${formatInteger(outputTotal)}`);
  }
  if (speedAverage != null) {
    metrics.push(`${speedLabel} ${speedAverage.toFixed(1)} t/s`);
  }
  if (cacheTotal != null) {
    metrics.push(`缓存 ${formatInteger(cacheTotal)}`);
  }
  if (latencyAverage != null) {
    metrics.push(`延时 ${Math.round(latencyAverage)} ms`);
  }

  return metrics;
}

function buildTurnMetricItems(turn: TurnTraceRecord) {
  const callModelEntries = getCachedTimeline(turn).filter((entry) => props.canonicalKind(entry.kind) === "call_model");
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

  const items: MetricDisplayItem[] = [];
  const inputTotal = inputs.length ? inputs.reduce((sum, value) => sum + value, 0) : turn.inputTokens ?? null;
  const cacheTotal = caches.length ? caches.reduce((sum, value) => sum + value, 0) : props.providerReturnedCacheHitInputTokens(turn);
  const outputTotal = outputs.length ? outputs.reduce((sum, value) => sum + value, 0) : turn.outputTokens ?? null;
  const speedAverage = outputTotal != null && generationDurations.length
    ? outputTotal / (generationDurations.reduce((sum, value) => sum + value, 0) / 1000)
    : tokenSpeed(turn);
  const speedLabel = latencies.length === generationDurations.length && generationDurations.length > 0
    ? "生成速度"
    : latencies.length === 0
      ? "整体速度"
      : "速度";
  const latencyAverage = latencies.length ? average(latencies) : turn.firstTokenLatencyMs ?? null;

  if (inputTotal != null) {
    items.push({ icon: ArrowUp, tooltip: "输入", value: formatInteger(inputTotal) });
  }
  if (outputTotal != null) {
    items.push({ icon: ArrowDown, tooltip: "输出", value: formatInteger(outputTotal) });
  }
  if (speedAverage != null) {
    items.push({ icon: Gauge, tooltip: speedLabel, value: `${speedAverage.toFixed(1)} t/s` });
  }
  if (cacheTotal != null) {
    items.push({ icon: Zap, tooltip: "缓存读取", value: formatInteger(cacheTotal) });
  }
  if (latencyAverage != null) {
    items.push({ icon: Clock3, tooltip: "首 token 延时", value: `${Math.round(latencyAverage)} ms` });
  }

  return items;
}

function buildTimelineMetricItems(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  if (props.canonicalKind(entry.kind) !== "call_model") {
    return [];
  }

  const metricEntry = timelineMetricEntry(turn, entry);
  const items: MetricDisplayItem[] = [];
  if (metricEntry.inputTokens != null) {
    items.push({ icon: ArrowUp, tooltip: "输入", value: formatInteger(metricEntry.inputTokens) });
  }
  if (metricEntry.outputTokens != null) {
    items.push({ icon: ArrowDown, tooltip: "输出", value: formatInteger(metricEntry.outputTokens) });
  }
  const tokenSpeed = formatEntryTokenSpeed(metricEntry);
  if (tokenSpeed) {
    items.push({ icon: Gauge, tooltip: formatTokenSpeedLabel(metricEntry), value: tokenSpeed });
  }
  if (metricEntry.cacheHitInputTokens != null) {
    items.push({ icon: Zap, tooltip: "缓存读取", value: formatInteger(metricEntry.cacheHitInputTokens) });
  }
  if (metricEntry.firstTokenLatencyMs != null) {
    items.push({ icon: Clock3, tooltip: "首 token 延时", value: `${metricEntry.firstTokenLatencyMs} ms` });
  }
  const durationMs =
    metricEntry.durationMs ??
    (typeof entry.durationMs === "number"
      ? entry.durationMs
      : typeof entry.turnDurationMs === "number"
        ? entry.turnDurationMs
        : null) ??
    (typeof metricEntry.durationSeconds === "number"
      ? Math.round(metricEntry.durationSeconds * 1000)
      : null);
  if (durationMs != null && durationMs > 0) {
    items.push({ icon: Clock3, tooltip: "耗时", value: formatDuration(durationMs / 1000) });
  }
  return items;
}

function timelineDurationText(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const kind = props.canonicalKind(entry.kind);
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
  return getCachedTimeline(turn).findIndex((candidate) => candidate.id === entryId);
}

function callModelOutputToolEntries(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  if (props.canonicalKind(entry.kind) !== "call_model") {
    return [];
  }

  const timeline = getCachedTimeline(turn);
  const entryIndex = timelineEntryIndex(turn, entry.id);
  if (entryIndex === -1) {
    return [];
  }

  const outputEntries: TraceTimelineEntry[] = [];
  for (let index = entryIndex + 1; index < timeline.length; index += 1) {
    const candidate = timeline[index]!;
    const kind = props.canonicalKind(candidate.kind);
    if (kind === "call_model") {
      break;
    }
    if (kind === "call_tool") {
      outputEntries.push(candidate);
    }
  }

  return outputEntries;
}

function shouldShowCallModelOutput(entry: TraceTimelineEntry) {
  return props.canonicalKind(entry.kind) === "call_model" && entry.state !== "active" && entry.state !== "pending";
}

function shouldShowCallModelReasoning(entry: TraceTimelineEntry) {
  return props.canonicalKind(entry.kind) === "call_model" && entry.state !== "active" && entry.state !== "pending";
}

function timelinePreviewText(_turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const kind = props.canonicalKind(entry.kind);
  if (kind === "call_tool") {
    const labels = (entry.toolActivities ?? [])
      .map((activity) => {
        const permissionBits = [
          permissionScopeLabel(activity) ? `权限: ${permissionScopeLabel(activity)}` : "",
          approvalLabel(activity) ? `审批: ${approvalLabel(activity)}` : "",
          permissionSourceLabel(activity) ? `来源: ${permissionSourceLabel(activity)}` : ""
        ].filter(Boolean).join(" · ");

        return [toolDisplayLabel(activity), permissionBits].filter(Boolean).join(" · ");
      })
      .filter((label, index, labelsList) => !!label && labelsList.indexOf(label) === index);
    return labels.join(" · ");
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
  const kind = props.canonicalKind(entry.kind);

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
    pushRow(rows, "模型", formatProviderModel(entry.providerName, entry.providerModel), { icon: Brain });
    pushRow(rows, "错误", entry.error, { multiline: true, tone: "danger" });
    return rows;
  }

  if (kind === "call_tool") {
    pushRow(rows, "错误", entry.error, { multiline: true, tone: "danger" });
    return rows;
  }

  return rows;
}

function buildTimelineDetailSections(turn: TurnTraceRecord, entry: TraceTimelineEntry) {
  const sections: TraceDetailSection[] = [];
  const kind = props.canonicalKind(entry.kind);

  if (kind === "build_context") {
    const buildContextObservation = entry.buildContextObservation ?? turn.buildContextObservation;
    const contextSections: Array<[string, string, string]> = [
      ["stable", "稳定前缀", buildContextText(buildContextObservation, "stablePrefixText")],
      ["semi", "半稳定上下文", buildContextText(buildContextObservation, "semiStableContextText")],
      ["volatile", "本轮输入", buildContextText(buildContextObservation, "volatileInputText")],
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

    // 将最终请求消息按单条消息拆分，每条独立折叠
    const messagesText = buildContextText(buildContextObservation, "requestMessagesText");
    if (messagesText) {
      const messageBlocks = parseRequestMessages(messagesText);
      for (const msg of messageBlocks) {
        const isLarge = msg.content.length > 500;
        sections.push({
          id: `msg-${msg.index}`,
          label: msg.header,
          content: msg.content,
          summary: isLarge ? previewInline(msg.content) : undefined,
          kind: "tool"
        });
      }
    }

    return sections;
  }

  if (kind === "call_tool") {
    for (const activity of entry.toolActivities ?? []) {
      sections.push({
        id: activity.id,
        label: toolDisplayLabel(activity),
        content: buildToolMessageDetail(activity),
        summary: activity.description,
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
        label: toolDisplayLabel(activity),
        content: buildToolMessageDetail(activity),
        summary: activity.description,
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

  const permissionScope = permissionScopeLabel(activity);
  const approvalMode = approvalLabel(activity);
  const permissionSource = permissionSourceLabel(activity);
  const failureKind = activity.capabilityInvocation?.failureKind;
  const failureLayer = activity.capabilityInvocation?.failureLayer;
  if (permissionScope || approvalMode || permissionSource) {
    lines.push(
      [
        permissionScope ? `权限: ${permissionScope}` : null,
        approvalMode ? `审批: ${approvalMode}` : null,
        permissionSource ? `来源: ${permissionSource}` : null
      ].filter(Boolean).join(" · ")
    );
  }

  if (failureKind) {
    lines.push(`失败分类: ${capabilityFailureLabel(failureKind)}`);
  }

  if (failureLayer) {
    lines.push(`失败层级: ${failureLayer}`);
  }

  return lines.join("\n\n");
}

function permissionScopeLabel(activity: ToolActivity) {
  return (
    activity.capabilityInvocation?.permissionFacts?.permissionScope
    || activity.capabilityInvocation?.permissionScope
    || ""
  ).trim();
}

function approvalLabel(activity: ToolActivity) {
  const facts = activity.capabilityInvocation?.permissionFacts;
  const mode = facts?.approvalMode?.trim();
  if (mode) {
    return mode;
  }

  const requiresApproval = facts?.requiresApproval ?? activity.capabilityInvocation?.requiresApproval;
  return requiresApproval ? "required" : "none";
}

function permissionSourceLabel(activity: ToolActivity) {
  return activity.capabilityInvocation?.permissionFacts?.decisionSource
    || activity.capabilityInvocation?.sourceKind
    || "";
}

function capabilityFailureLabel(failureKind?: string | null) {
  switch (failureKind) {
    case "permission_denied":
      return "权限拒绝 (permission_denied)";
    case "source_unavailable":
      return "来源不可用 (source_unavailable)";
    case "out_of_scope":
      return "超出作用域 (out_of_scope)";
    case "invocation_failed":
      return "调用失败 (invocation_failed)";
    case "malformed_response":
      return "响应异常 (malformed_response)";
    case "capability_not_found":
      return "能力不存在 (capability_not_found)";
    default:
      return failureKind || "";
  }
}

function toolDisplayLabel(activity: {
  displayNameZh?: string | null;
  canonicalToolName?: string | null;
  name: string;
}) {
  return activity.displayNameZh?.trim()
    || activity.canonicalToolName?.trim()
    || activity.name;
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
  for (const section of buildTimelineDetailSections(turn, entry)) {
    if (section.content) {
      lines.push("", `--- ${section.label} ---`, section.content);
    }
  }
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

  getCachedTimeline(turn).forEach((entry) => {
    parts.push(buildTimelineCopyText(turn, entry));
  });

  return parts.join("\n\n");
}

function toggleTurn(turnId: string) {
  activeTurnId.value = activeTurnId.value === turnId ? "" : turnId;
  activeTraceStepKey.value = "";
}

function toggleTraceStep(turnId: string, stepId: string) {
  const key = `${turnId}:${stepId}`;
  if (activeTraceStepKey.value === key) {
    activeTraceStepKey.value = "";
    activeTraceDetailKey.value = "";
    return;
  }

  activeTraceStepKey.value = key;
  activeTraceDetailKey.value = "";

  const turn = props.turns.find((item) => item.turnId === turnId);
  const entry = props.turnTimeline(turn ?? ({} as TurnTraceRecord)).find((item) => item.id === stepId);
  if (!turn || !entry || props.canonicalKind(entry.kind) !== "call_model") {
    return;
  }

  const firstToolDetail = buildTimelineDetailSections(turn, entry).find((section) => section.kind === "tool");
  if (firstToolDetail) {
    activeTraceDetailKey.value = traceDetailKey(turnId, stepId, firstToolDetail.id);
  }
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

watch(() => props.sessionId, () => {
  activeTurnId.value = "";
  activeTraceStepKey.value = "";
  activeTraceDetailKey.value = "";
  expandedResultKeys.value = [];
});

watch(orderedTurnTraceSignature, () => {
  const turns = props.turns;
  if (!turns.some((turn) => turn.turnId === activeTurnId.value)) {
    activeTurnId.value = turns[turns.length - 1]?.turnId ?? "";
    activeTraceStepKey.value = "";
    activeTraceDetailKey.value = "";
  }
});
</script>

<template>
  <section class="collapsible-shell border-b border-stone-200/60 pb-4" :data-open="open">
    <button class="flex w-full items-center justify-between gap-3 text-left" type="button" data-testid="trace-panel-toggle" @click="emit('toggle')">
      <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
        <Clock3 class="h-3.5 w-3.5" />
        <span>Trace</span>
      </div>
      <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': open }" />
    </button>

    <div v-if="open" class="collapsible-body">
      <ScrollArea
        ref="traceBodyScrollRef"
        class="trace-body-scroll max-h-[24rem] min-h-[3rem]"
        viewport-class="trace-body-viewport"
        @vue:mounted="onTraceBodyMounted"
        @vue:unmounted="onTraceBodyUnmounted"
      >
        <section class="collapsible-content mt-2">
          <div :style="{ height: `${virtualPaddingTop}px` }" aria-hidden="true"></div>
          <div class="space-y-1">
            <section
              v-for="turn in visibleTurns"
              :key="turn.turnId"
              class="collapsible-shell overflow-hidden py-1.5"
              :data-open="activeTurnId === turn.turnId"
            >
          <button class="group flex w-full items-start justify-between gap-2 text-left" type="button" @click="toggleTurn(turn.turnId)">
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
              <div v-if="buildTurnMetricItems(turn).length" class="pl-[1.125rem] flex flex-nowrap items-center gap-x-3 text-[10px] leading-[1.15] text-stone-400">
                <Tooltip v-for="item in buildTurnMetricItems(turn)" :key="item.tooltip" :text="item.tooltip">
                  <span class="inline-flex items-center gap-0.5 whitespace-nowrap">
                    <component :is="item.icon" class="h-3 w-3" />
                    {{ item.value }}
                  </span>
                </Tooltip>
              </div>
            </div>
            <div class="flex items-center gap-1">
              <span v-if="turnDurationText(turn)" class="shrink-0 whitespace-nowrap text-[10px] leading-[1.3] text-stone-400">
                {{ turnDurationText(turn) }}
              </span>
              <button
                class="invisible group-hover:visible inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                type="button"
                @click.stop="emit('copy', turnCopyKey(turn.turnId), buildTurnCopyText(turn))"
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
                v-for="entry in getCachedTimeline(turn)"
                :key="entry.id"
                class="collapsible-shell overflow-hidden py-0.5"
                :data-open="activeTraceStepKey === turnStepKey(turn.turnId, entry.id)"
              >
                <button
                  class="group flex w-full items-start justify-between gap-1.5 text-left"
                  type="button"
                  :data-testid="`trace-step-button-${entry.id}`"
                  :disabled="isCheckpointPersistEntry(entry)"
                  @click="!isCheckpointPersistEntry(entry) && toggleTraceStep(turn.turnId, entry.id)"
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
                      <div class="flex flex-nowrap items-center gap-x-3">
                        <Tooltip
                          v-for="item in buildTimelineMetricItems(turn, entry)"
                          :key="`${turn.turnId}-${entry.id}-${item.tooltip}`"
                          :text="item.tooltip"
                        >
                          <span class="inline-flex items-center gap-0.5 whitespace-nowrap text-stone-400">
                            <component :is="item.icon" class="h-3 w-3" />
                            {{ item.value }}
                          </span>
                        </Tooltip>
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
                      v-if="!isCheckpointPersistEntry(entry)"
                      class="invisible group-hover:visible inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                      type="button"
                      @click.stop="emit('copy', traceCopyKey(turn.turnId, entry.id), buildTimelineCopyText(turn, entry))"
                    >
                      <component :is="copiedKey === traceCopyKey(turn.turnId, entry.id) ? Check : Copy" class="h-3 w-3" />
                    </button>
                    <ChevronRight
                      v-if="!isCheckpointPersistEntry(entry)"
                      class="h-3 w-3 shrink-0 text-stone-300 transition duration-200"
                      :class="{ 'rotate-90': activeTraceStepKey === turnStepKey(turn.turnId, entry.id) }"
                    />
                  </div>
                </button>

                <div v-if="!isCheckpointPersistEntry(entry) && activeTraceStepKey === turnStepKey(turn.turnId, entry.id)" class="collapsible-body-open">
                  <div class="collapsible-content mt-1 pl-4">
                    <section>
                      <div class="space-y-1">
                        <div
                          v-for="row in buildTimelineRows(turn, entry)"
                          :key="turn.turnId + '-' + entry.id + '-' + row.label"
                          class="overflow-x-auto text-[10px] leading-[1.35]"
                        >
                          <div class="flex min-w-0 items-start gap-2">
                            <template v-if="row.icon">
                              <Tooltip :text="row.label">
                                <span class="inline-flex items-center gap-1 shrink-0 whitespace-nowrap text-stone-400">
                                  <component :is="row.icon" class="h-3 w-3" />
                                  <span>{{ row.value }}</span>
                                </span>
                              </Tooltip>
                            </template>
                            <template v-else>
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
                            </template>
                          </div>
                        </div>
                      </div>
                    </section>

                    <template v-for="section in buildTimelineDetailSections(turn, entry)" :key="traceDetailKey(turn.turnId, entry.id, section.id)">
                      <section
                        v-if="section.kind === 'model'"
                        class="mt-2 border-l border-stone-200/80 pl-2"
                      >
                        <div class="group flex items-start justify-between gap-1.5 py-0.5">
                          <div class="min-w-0 text-[10px] uppercase tracking-[0.14em] text-stone-400">
                            {{ section.label }}
                          </div>
                          <button
                            class="invisible group-hover:visible inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                            type="button"
                            :data-testid="`trace-detail-button-${entry.id}-${section.id}`"
                            @click.stop="emit('copy', traceDetailKey(turn.turnId, entry.id, section.id), section.content)"
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
                          class="group flex w-full items-start justify-between gap-1 py-0 text-left"
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
                                class="invisible group-hover:visible inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                                type="button"
                                @click.stop="emit('copy', traceDetailKey(turn.turnId, entry.id, section.id), section.content)"
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

                        <div v-if="activeTraceDetailKey === traceDetailKey(turn.turnId, entry.id, section.id)" class="collapsible-body-open">
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
          </div>
          <div :style="{ height: `${virtualPaddingBottom}px` }" aria-hidden="true"></div>
        </section>
      </ScrollArea>
    </div>
  </section>
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

.collapsible-body-open {
  animation: collapseFadeIn 200ms ease-out;
}

@keyframes collapseFadeIn {
  from {
    opacity: 0;
    transform: translateY(-3px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
</style>
