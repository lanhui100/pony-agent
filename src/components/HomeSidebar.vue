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
import type { TraceStep, TurnTraceRecord } from "@/types/runtime";
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
  sessionId,
  sessionSummary,
  totalTokens,
  turnTraceHistory
} = storeToRefs(runtimeStore);

const activePanel = ref<"status" | "trace" | "tools" | "">("trace");
const activeTurnId = ref("");
const activeTraceStepKey = ref("");
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
  const metrics = [];

  if (turn.totalTokens != null) {
    metrics.push(`${turn.totalTokens} tok`);
  }

  if (turn.firstTokenLatencyMs != null) {
    metrics.push(`首 token ${turn.firstTokenLatencyMs} ms`);
  }

  return metrics.join(" 路 ");
}

function detailText(turn: TurnTraceRecord) {
  if (turn.error) {
    return turn.error;
  }

  if (turn.fallbackReason) {
    return turn.fallbackReason;
  }

  return turn.sessionSummary || "";
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

function expandedResultKey(turnId: string, stepId: string, label: string) {
  return `${turnId}:${stepId}:${label}`;
}

function stepDurationText(turn: TurnTraceRecord, step: TraceStep) {
  if (step.id === "step-call-model" && turn.firstTokenLatencyMs != null) {
    return `${turn.firstTokenLatencyMs} ms`;
  }

  if (step.id === "step-call-tool" && turn.toolActivities.length) {
    const totalSeconds = turn.toolActivities.reduce((sum, tool) => sum + (tool.durationSeconds ?? 0), 0);
    return totalSeconds > 0 ? formatDuration(totalSeconds) : "";
  }

  return "";
}

function stepTokenStats(turn: TurnTraceRecord, step: TraceStep) {
  const stats: string[] = [];

  if ((step.id === "step-plan" || step.id === "step-call-model") && turn.inputTokens != null) {
    stats.push(`IN:${turn.inputTokens}`);
  }

  if ((step.id === "step-call-model" || step.id === "step-return") && turn.outputTokens != null) {
    stats.push(`OUT:${turn.outputTokens}`);
  }

  return stats;
}

function buildActualToolDetailText(turn: TurnTraceRecord) {
  if (!turn.toolActivities.length) {
    return "";
  }

  return turn.toolActivities
    .map((tool, index) => {
      const lines = [`#${index + 1} ${tool.name}`];

      if (tool.summary.trim()) {
        lines.push(`摘要: ${tool.summary.trim()}`);
      }

      if (tool.durationSeconds != null) {
        lines.push(`耗时: ${formatDuration(tool.durationSeconds)}`);
      }

      if (tool.argumentsText?.trim()) {
        lines.push(`参数: ${tool.argumentsText.trim()}`);
      }

      if (tool.resultText?.trim()) {
        lines.push(`${tool.status === "error" ? "错误" : "结果"}: ${tool.resultText.trim()}`);
      }

      return lines.join("\n");
    })
    .join("\n\n");
}

function buildPendingStepText(step: TraceStep) {
  if (step.id === "step-call-tool") {
    return "本轮未触发工具调用。";
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

function buildStepRows(turn: TurnTraceRecord, step: TraceStep) {
  const userMessage = turnUserMessage(turn.turnId);
  const assistantMessage = turnAssistantMessage(turn.turnId);
  const rows: DetailRow[] = [];

  if (step.id === "step-plan") {
    if (hasText(userMessage?.content)) {
      pushRow(rows, "输入内容", userMessage!.content.trim(), {
        multiline: true,
        inputKind: detectInputKind(userMessage!.content)
      });
    }
  }

  if (step.id === "step-context") {
    pushRow(rows, "Requested", turn.providerRequestedName);
    pushRow(rows, "Provider", turn.providerName);
    pushRow(rows, "Protocol", turn.providerProtocol);
    pushRow(rows, "Model", turn.providerModel);
    pushRow(rows, "Source", turn.providerSource);
    pushRow(rows, "Mode", turn.providerMode);
    pushRow(rows, "Session", turn.sessionSummary, { multiline: true });
    pushRow(rows, "Fallback", turn.fallbackReason, { multiline: true, tone: "warning" });
  }

  if (step.id === "step-call-model") {
    pushRow(rows, "模型", providerModelLabel(turn));
    pushRow(rows, "来源", turn.providerSource);
    pushRow(rows, "协议", turn.providerProtocol);
    pushRow(rows, "模式", turn.providerMode);
    pushRow(rows, "耗时", stepDurationText(turn, step));
    pushRow(rows, "状态", step.state === "pending" ? buildPendingStepText(step) : null, { tone: "muted" });
    pushRow(rows, "错误", buildStepErrorText(turn, step), { multiline: true, tone: "danger" });
  }

  if (step.id === "step-call-tool") {
    pushRow(rows, "耗时", stepDurationText(turn, step));
    pushRow(rows, "状态", step.state === "pending" ? buildPendingStepText(step) : null, { tone: "muted" });
    pushRow(rows, "调用详情", buildActualToolDetailText(turn), {
      multiline: true,
      tone: turn.toolActivities.some((tool) => tool.status === "error") ? "danger" : "default"
    });
    pushRow(rows, "错误", buildStepErrorText(turn, step), { multiline: true, tone: "danger" });
  }

  if (step.id === "step-return") {
    pushRow(rows, "状态", step.state === "pending" ? buildPendingStepText(step) : null, { tone: "muted" });
    pushRow(rows, "结果", assistantMessage?.content ?? "", {
      multiline: true,
      expandable: true
    });
    pushRow(rows, "错误", buildStepErrorText(turn, step), { multiline: true, tone: "danger" });
  }

  return rows;
}

function buildCopyText(turn: TurnTraceRecord, step: TraceStep) {
  return [
    `${turn.title} / ${step.label}`,
    ...buildStepRows(turn, step).map((row) => `${row.label}: ${row.value}`)
  ].join("\n\n");
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
      return;
    }

    activeTurnId.value = turnId;
    activeTraceStepKey.value = "";
  },
  { immediate: true }
);

watch(sessionId, () => {
  activeTurnId.value = "";
  activeTraceStepKey.value = "";
  copiedKey.value = "";
  expandedResultKeys.value = [];
});

watch(
  orderedTurnTraces,
  (turns) => {
    if (!turns.some((turn) => turn.turnId === activeTurnId.value)) {
      activeTurnId.value = turns[turns.length - 1]?.turnId ?? "";
      activeTraceStepKey.value = "";
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
          <button class="flex w-full items-center justify-between gap-3 text-left" type="button" @click="togglePanel('status')">
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
                  <span class="text-stone-400">闃舵</span>
                  <span class="text-right text-stone-800">{{ phaseLabel }}</span>
                </div>
                <div v-if="displayModel" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">妯″瀷</span>
                  <span class="break-words text-right text-stone-800 [overflow-wrap:anywhere]">{{ displayModel }}</span>
                </div>
                <div v-if="providerProtocol" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">鍗忚</span>
                  <span class="text-right text-stone-800">{{ providerProtocol }}</span>
                </div>
                <div v-if="providerMode" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">妯″紡</span>
                  <span class="text-right text-stone-800">{{ providerMode }}</span>
                </div>
                <div v-if="totalTokens != null" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">鎬?token</span>
                  <span class="text-right text-stone-800">{{ totalTokens }}</span>
                </div>
                <div v-if="firstTokenLatencyMs != null" class="flex items-start justify-between gap-3">
                  <span class="text-stone-400">棣?token</span>
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
          <button class="flex w-full items-center justify-between gap-3 text-left" type="button" @click="togglePanel('tools')">
            <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
              <Wrench class="h-3.5 w-3.5" />
              <span>Tools</span>
            </div>
            <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': activePanel === 'tools' }" />
          </button>

          <div class="collapsible-body">
            <section class="collapsible-content mt-3 space-y-2">
              <section
                v-for="tool in availableTools"
                :key="tool.name"
                class="collapsible-shell overflow-hidden rounded-[0.45rem] bg-[#faf7f1] px-3 py-2"
                :data-open="activeTraceStepKey === toolPanelKey(tool.name)"
              >
                <button
                  class="flex w-full items-start justify-between gap-3 text-left"
                  type="button"
                  @click="activeTraceStepKey = activeTraceStepKey === toolPanelKey(tool.name) ? '' : toolPanelKey(tool.name)"
                >
                  <div class="min-w-0">
                    <div class="text-[12px] font-medium text-stone-800">{{ tool.name }}</div>
                    <p class="mt-1 text-[11px] leading-4 text-stone-500">
                      {{ tool.description }}
                    </p>
                  </div>
                  <ChevronRight
                    class="mt-0.5 h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200"
                    :class="{ 'rotate-90': activeTraceStepKey === toolPanelKey(tool.name) }"
                  />
                </button>
                <div class="collapsible-body">
                  <div class="collapsible-content mt-2">
                    <section class="rounded-[0.35rem] bg-white/72 px-2.5 py-2">
                      <div class="space-y-2 text-[11px] leading-4">
                        <div class="space-y-1">
                          <div class="text-stone-400">鍙傛暟</div>
                          <div class="text-stone-700">{{ toolInputSummary(tool.name) }}</div>
                        </div>
                        <div class="space-y-1">
                          <div class="text-stone-400">蹇呭～</div>
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
          <button class="flex w-full items-center justify-between gap-3 text-left" type="button" @click="togglePanel('trace')">
            <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
              <Clock3 class="h-3.5 w-3.5" />
              <span>Trace</span>
            </div>
            <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': activePanel === 'trace' }" />
          </button>

          <div class="collapsible-body">
            <section class="collapsible-content mt-3 space-y-2">
              <section
                v-for="turn in orderedTurnTraces"
                :key="turn.turnId"
                class="collapsible-shell overflow-hidden rounded-[0.45rem] bg-[#faf7f1] px-2.5 py-2"
                :data-open="activeTurnId === turn.turnId"
              >
                <button class="flex w-full items-start justify-between gap-3 text-left" type="button" @click="toggleTurn(turn.turnId)">
                  <div class="min-w-0 space-y-1">
                    <div class="flex items-center gap-2 text-[13px] text-stone-800">
                      <component
                        :is="turnStateIcon(turn)"
                        class="h-3.5 w-3.5 shrink-0"
                        :class="{
                          'animate-spin text-stone-500': turn.phase === 'calling_model' || turn.phase === 'calling_tool',
                          'text-rose-600': turn.phase === 'failed' || !!turn.error,
                          'text-stone-700': turn.phase === 'completed' && !turn.error,
                          'text-stone-500': turn.phase !== 'failed' && turn.phase !== 'completed' && !turn.error
                        }"
                      />
                      <span class="truncate">{{ turn.title }}</span>
                    </div>
                    <div v-if="turnMeta(turn)" class="text-[11px] leading-4 text-stone-400">
                      {{ turnMeta(turn) }}
                    </div>
                  </div>
                  <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': activeTurnId === turn.turnId }" />
                </button>

                <div class="collapsible-body">
                  <div class="collapsible-content mt-2 space-y-1.5 pl-1">
                    <p v-if="detailText(turn)" class="break-words text-[11px] leading-4 text-stone-600 [overflow-wrap:anywhere]">
                      {{ detailText(turn) }}
                    </p>

                    <section
                      v-for="step in turn.traceSteps"
                      :key="step.id"
                      class="collapsible-shell overflow-hidden rounded-[0.4rem] bg-white/78 px-2 py-1.5"
                :data-open="activeTraceStepKey === turnStepKey(turn.turnId, step.id)"
                    >
                      <button
                        class="flex w-full items-start justify-between gap-2 text-left"
                        type="button"
                        @click="toggleTraceStep(turn.turnId, step.id)"
                      >
                        <div class="min-w-0 space-y-1">
                          <div class="flex min-w-0 items-center gap-2 text-[11px] leading-4 text-stone-700">
                            <component
                              :is="traceStateIcon(step.state)"
                              class="h-3.5 w-3.5 shrink-0"
                              :class="{
                                'text-stone-500': step.state === 'completed',
                                'animate-spin text-stone-500': step.state === 'active',
                                'text-rose-600': step.state === 'error',
                                'text-stone-300': step.state === 'pending'
                              }"
                            />
                            <span class="truncate">{{ step.label }}</span>
                          </div>
                          <div class="flex flex-wrap items-center gap-1.5 text-[10px] text-stone-400">
                            <span
                              v-for="stat in stepTokenStats(turn, step)"
                                :key="turn.turnId + '-' + step.id + '-' + stat"
                              class="rounded-full bg-[#f4ede1] px-2 py-0.5 text-stone-500"
                            >
                              {{ stat }}
                            </span>
                            <span v-if="stepDurationText(turn, step)" class="rounded-full bg-[#f4ede1] px-2 py-0.5 text-stone-500">
                              {{ stepDurationText(turn, step) }}
                            </span>
                          </div>
                        </div>

                        <div class="flex items-center gap-1.5">
                          <button
                            class="inline-flex h-6 w-6 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
                            type="button"
                            @click.stop="copyText(traceCopyKey(turn.turnId, step.id), buildCopyText(turn, step))"
                          >
                            <component :is="copiedKey === traceCopyKey(turn.turnId, step.id) ? Check : Copy" class="h-3.5 w-3.5" />
                          </button>
                          <ChevronRight
                            class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200"
                            :class="{ 'rotate-90': activeTraceStepKey === turnStepKey(turn.turnId, step.id) }"
                          />
                        </div>
                      </button>

                      <div class="collapsible-body">
                        <div class="collapsible-content mt-2">
                          <section class="rounded-[0.35rem] bg-[#fbf8f3] px-2.5 py-2">
                            <div class="space-y-2">
                              <div
                                v-for="row in buildStepRows(turn, step)"
                                :key="turn.turnId + '-' + step.id + '-' + row.label"
                                class="space-y-1 text-[11px] leading-4"
                              >
                                <div class="flex items-center justify-between gap-3">
                                  <span class="text-stone-400">{{ row.label }}</span>
                                  <component
                                    v-if="row.inputKind"
                                    :is="inputKindIcon(row.inputKind)"
                                    class="h-3.5 w-3.5 shrink-0 text-stone-400"
                                  />
                                </div>
                                <div
                                  class="min-w-0 break-words [overflow-wrap:anywhere]"
                                  :class="[rowToneClass(row.tone), row.multiline ? 'whitespace-pre-wrap text-left' : 'text-right']"
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
