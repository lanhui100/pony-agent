<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { usePlanStore } from "@/stores/plan";
import { useGoalStore } from "@/stores/goal";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import PlanPanel from "@/components/PlanPanel.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import { clearTraceProjectionMemo } from "@/lib/runtime/trace-projection";
import { useCopyFeedback, useTraceProjection } from "@/lib/runtime/useTraceProjection";

/**
 * PA-096：右侧栏为纯对话过程面板（状态 / 计划）。
 *
 * - Tools 目录位于配置页（ConfigToolsSection）；Trace 位于观测页（TraceInspector）。
 * - 状态面板消费活跃/最新调用指标，在每次模型调用发生时即时刷新。
 */

const runtimeStore = useRuntimeStore();
const planStore = usePlanStore();
const goalStore = useGoalStore();
const hasPlanOrTasks = computed(
  () => planStore.plans.length > 0 || planStore.todos.length > 0 || goalStore.hasGoal
);

const {
  error,
  fallbackReason,
  isSubmitting,
  sessionError,
  sessionId,
  sessionOperation
} = storeToRefs(runtimeStore);

const activePanel = ref<"plan" | "">("");
const { copiedKey, copyText, resetCopy, dispose } = useCopyFeedback();

// 状态面板消费活跃调用指标（liveTurnEnabled: true），每次模型调用产生时即刻更新
const {
  contextDisplayTokens,
  currentContextWindowTokens,
  sessionCacheHitRatio,
  sessionCacheHitTokensTotal,
  sessionInputTokensTotal,
  sessionModelCallCount,
  sessionOutputTokensTotal,
  sessionToolCallCount,
  sessionTokenGenerationSpeed,
  sessionTurnCount,
  showContextUsage
} = useTraceProjection({ liveTurnEnabled: ref(true) });

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

function togglePlan() {
  activePanel.value = activePanel.value === "plan" ? "" : "plan";
}

watch(
  sessionId,
  () => {
    resetCopy();
    clearTraceProjectionMemo();
  }
);

onBeforeUnmount(() => {
  dispose();
  clearTraceProjectionMemo();
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
          :token-generation-speed="sessionTokenGenerationSpeed"
          :show-context-usage="showContextUsage"
          :context-display-tokens="contextDisplayTokens"
          :context-window-tokens="currentContextWindowTokens"
          :status-summary="sessionStatusSummary"
          :fallback-reason="fallbackReason"
          :error="error"
          @copy-session-id="copyText('session-id', sessionId)"
        />

        <PlanPanel
          v-if="hasPlanOrTasks"
          :session-id="sessionId"
          :open="activePanel === 'plan'"
          @toggle="togglePlan()"
        />
      </div>
    </ScrollArea>
  </aside>
</template>
