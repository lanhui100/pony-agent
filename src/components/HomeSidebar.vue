<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { usePlanStore } from "@/stores/plan";
import { useGoalStore } from "@/stores/goal";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import PlanPanel from "@/components/PlanPanel.vue";
import TeamRosterPanel from "@/components/team/TeamRosterPanel.vue";
import TeamTaskBoardPanel from "@/components/team/TeamTaskBoardPanel.vue";
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
  sessionId
} = storeToRefs(runtimeStore);

const activePanel = ref<"plan" | "roster" | "tasks" | "">("");
const { copiedKey, copyText, resetCopy, dispose } = useCopyFeedback();

// 状态面板消费活跃调用指标（liveTurnEnabled: true），每次模型调用产生时即刻更新
const {
  contextDisplayTokens,
  currentContextWindowTokens,
  sessionCacheHitRatio,
  sessionCacheHitTokensTotal,
  sessionModelCallCount,
  sessionOutputTokensTotal,
  sessionToolCallCount,
  sessionTokenGenerationSpeed,
  tokenSpeedCalculationMode,
  tokenSpeedDebugInfo,
  sessionTurnCount,
  showContextUsage
} = useTraceProjection({ liveTurnEnabled: ref(true) });

function togglePlan() {
  activePanel.value = activePanel.value === "plan" ? "" : "plan";
}

function toggleRoster() {
  activePanel.value = activePanel.value === "roster" ? "" : "roster";
}

function toggleTasks() {
  activePanel.value = activePanel.value === "tasks" ? "" : "tasks";
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
        <!-- Wave-15 S1：状态栏「输入」= 当前上下文占用（最近一次模型调用）；S2 会话累计口径保留在 sessionInputTokensTotal。 -->
        <HomeStatusPanel
          :session-id="sessionId"
          :turn-count="sessionTurnCount"
          :model-call-count="sessionModelCallCount"
          :tool-call-count="sessionToolCallCount"
          :copied="copiedKey === 'session-id'"
          :input-tokens-total="contextDisplayTokens"
          :output-tokens-total="sessionOutputTokensTotal"
          :cache-hit-tokens-total="sessionCacheHitTokensTotal"
          :cache-hit-ratio="sessionCacheHitRatio"
          :token-generation-speed="sessionTokenGenerationSpeed"
          :token-speed-calculation-mode="tokenSpeedCalculationMode"
          :speed-debug-info="tokenSpeedDebugInfo"
          :show-context-usage="showContextUsage"
          :context-display-tokens="contextDisplayTokens"
          :context-window-tokens="currentContextWindowTokens"
          :class="hasPlanOrTasks ? 'border-b border-stone-200/70 pb-4' : ''"
          @copy-session-id="copyText('session-id', sessionId)"
        />

        <!-- Wave-1 C6：计划面板常驻（不再以「已有数据」为渲染前提），空态由 PlanPanel 内部展示。 -->
        <PlanPanel
          :session-id="sessionId"
          :open="activePanel === 'plan'"
          @toggle="togglePlan()"
        />

        <TeamRosterPanel :open="activePanel === 'roster'" @toggle="toggleRoster()" />
        <TeamTaskBoardPanel :open="activePanel === 'tasks'" @toggle="toggleTasks()" />
      </div>
    </ScrollArea>
  </aside>
</template>
