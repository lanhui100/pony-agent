<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import type { TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";
import { useRuntimeStore } from "@/stores/runtime";
import { useProviderStore } from "@/stores/providers";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import HomeToolsPanel from "@/components/HomeToolsPanel.vue";
import HomeTracePanel from "@/components/HomeTracePanel.vue";
import PlanPanel from "@/components/PlanPanel.vue";
import DebugPanel from "@/components/DebugPanel.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();

const {
  activeTurnId: runtimeActiveTurnId,
  availableTools,
  error,
  fallbackReason,
  firstTokenLatencyMs,
  inputTokens,
  isSubmitting,
  messages,
  outputTokens,
  phase,
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

const activePanel = ref<"tools" | "trace" | "plan" | "debug" | "">("trace");
const copiedKey = ref("");
let copiedTimer: number | null = null;

const retrievedSessionContext = computed(() => retrievedContext.value?.sessionContext ?? null);
const sessionStatusSummary = computed(() => {
  if (sessionOperation.value === "initializing") {
    return "正在加载最近对话…";
  }

  if (sessionOperation.value === "switching") {
    return "正在切换对话…";
  }

  if (sessionOperation.value === "deleting") {
    return "正在删除对话…";
  }

  if (sessionError.value?.trim()) {
    return sessionError.value.trim();
  }

  if (isSubmitting.value) {
    return "正在等待回复…";
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

  // Backward search instead of full reverse+find — avoids copying the entire array
  const allMessages = messages.value;
  let latestUserMessage: typeof allMessages[number] | undefined;
  for (let i = allMessages.length - 1; i >= 0; i--) {
    const msg = allMessages[i]!;
    if (msg.turnId === turnId && msg.role === "user") {
      latestUserMessage = msg;
      break;
    }
  }

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
const contextDisplayTokens = computed(() => {
  if (inputTokens.value != null) return inputTokens.value;
  if (isSubmitting.value && traceTimeline.value?.length) {
    for (let i = traceTimeline.value.length - 1; i >= 0; i--) {
      const entry = traceTimeline.value[i];
      if (canonicalTraceTimelineKind(entry.kind) === "call_model" && entry.inputTokens != null) {
        return entry.inputTokens;
      }
    }
  }
  for (let i = turnTraceHistory.value.length - 1; i >= 0; i--) {
    const turn = turnTraceHistory.value[i];
    if (turn.turnId !== runtimeActiveTurnId.value && turn.inputTokens != null) {
      return turn.inputTokens;
    }
  }
  return latestTurn.value?.inputTokens ?? null;
});
const currentContextWindowTokens = computed(
  () => retrievedSessionContext.value?.contextWindowTokens ?? providerStore.currentModel?.capabilities?.contextWindowTokens ?? null
);
const sessionTurnCount = computed(() => orderedTurnTraces.value.length);
// Cached timeline per turn — computed once per orderedTurnTraces change,
// shared by all consumers (template v-for, session stats, metric helpers).
const turnTimelineCache = computed(() => {
  const cache = new Map<string, TraceTimelineEntry[]>();
  for (const turn of orderedTurnTraces.value) {
    cache.set(turn.turnId, turnTimeline(turn));
  }
  return cache;
});
const sessionModelCallCount = computed(() => {
  const cache = turnTimelineCache.value;
  let sum = 0;
  for (const entries of cache.values()) {
    for (const entry of entries) {
      if (canonicalTraceTimelineKind(entry.kind) === "call_model") {
        sum++;
      }
    }
  }
  return sum;
});
const sessionToolCallCount = computed(() => {
  const cache = turnTimelineCache.value;
  let sum = 0;
  for (const entries of cache.values()) {
    for (const entry of entries) {
      if (canonicalTraceTimelineKind(entry.kind) === "call_tool") {
        sum++;
      }
    }
  }
  return sum;
});
const sessionInputTokensTotal = computed(() =>
  orderedTurnTraces.value.reduce((sum, turn) => sum + (turn.inputTokens ?? 0), 0)
);
const sessionCacheHitTokensTotal = computed(() =>
  orderedTurnTraces.value.reduce((sum, turn) => sum + (providerReturnedCacheHitInputTokens(turn) ?? 0), 0)
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
const showContextUsage = computed(() => !!latestTurn.value);

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

function turnTimeline(turn: TurnTraceRecord) {
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

function providerReturnedCacheHitInputTokens(turn: TurnTraceRecord) {
  const values = (turn.providerCallRecords ?? [])
    .map((record) => record.cacheHitInputTokens)
    .filter((value): value is number => typeof value === "number" && Number.isFinite(value));

  return values.length ? values.reduce((sum, value) => sum + value, 0) : null;
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

function togglePanel(panel: "tools" | "trace" | "plan") {
  activePanel.value = activePanel.value === panel ? "" : panel;
}

watch(sessionId, () => {
  copiedKey.value = "";
});
</script>

<template>
  <aside class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.6rem] border border-stone-200/70 bg-white/62">
    <ScrollArea class="min-h-0 flex-1" viewport-class="px-4 pt-10 pb-4">
      <div class="flex min-h-full flex-col gap-3">
        <HomeStatusPanel
          :session-id="sessionId"
          :turn-count="sessionTurnCount"
          :model-call-count="sessionModelCallCount"
          :tool-call-count="sessionToolCallCount"
          :copied="copiedKey === 'session-id'"
          :input-tokens-total="sessionInputTokensTotal"
          :output-tokens-total="sessionOutputTokensTotal"
          :cache-hit-tokens-total="sessionCacheHitTokensTotal"
          :cache-hit-ratio="sessionCacheHitRatio"
          :show-context-usage="showContextUsage"
          :context-display-tokens="contextDisplayTokens"
          :context-window-tokens="currentContextWindowTokens"
          :status-summary="sessionStatusSummary"
          :fallback-reason="fallbackReason"
          :error="error"
          @copy-session-id="copyText('session-id', sessionId)"
        />

        <HomeToolsPanel
          :tools="availableTools"
          :open="activePanel === 'tools'"
          @toggle="togglePanel('tools')"
        />

        <HomeTracePanel
          :turns="orderedTurnTraces"
          :session-id="sessionId"
          :copied-key="copiedKey"
          :open="activePanel === 'trace'"
          :canonical-kind="canonicalTraceTimelineKind"
          :turn-timeline="turnTimeline"
          :provider-returned-cache-hit-input-tokens="providerReturnedCacheHitInputTokens"
          @copy="copyText"
          @toggle="togglePanel('trace')"
        />

        <PlanPanel
          :session-id="sessionId"
          :open="activePanel === 'plan'"
          @toggle="togglePanel('plan')"
        />

        <DebugPanel
          :active="activePanel === 'debug'"
          @toggle="activePanel = activePanel === 'debug' ? '' : 'debug'"
        />
      </div>
    </ScrollArea>
  </aside>
</template>
