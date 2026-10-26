<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { usePlanStore } from "@/stores/plan";
import { useGoalStore } from "@/stores/goal";
import { useTeamStore } from "@/stores/team";
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
const teamStore = useTeamStore();

// 计划与任务在有值时显示（有 plan、todo 或 goal）
const hasPlanOrTasks = computed(
  () => planStore.plans.length > 0 || planStore.todos.length > 0 || goalStore.hasGoal
);

// 团队在有子智能体时才显示（成员数 > 1，或排除 lead 后的子代理数 > 0）
const hasSubagents = computed(() => {
  return teamStore.members.some((m) => m.target !== "lead") || teamStore.members.length > 1;
});

// 任务看板在有任务时显示
const hasTeamTasks = computed(() => teamStore.tasks.length > 0);

const hasVisiblePanelsBelowStatus = computed(
  () => hasPlanOrTasks.value || hasSubagents.value || hasTeamTasks.value
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
          :class="hasVisiblePanelsBelowStatus ? 'border-b border-stone-200/70 pb-4' : ''"
          @copy-session-id="copyText('session-id', sessionId)"
        />

        <!-- 计划与任务：在有值时显示 -->
        <PlanPanel
          v-if="hasPlanOrTasks"
          :session-id="sessionId"
          :open="activePanel === 'plan'"
          @toggle="togglePlan()"
        />

        <!-- 团队：改为「团队」，在有子智能体时才显示 -->
        <TeamRosterPanel
          v-if="hasSubagents"
          :open="activePanel === 'roster'"
          @toggle="toggleRoster()"
        />

        <!-- 任务看板：在有值时显示 -->
        <TeamTaskBoardPanel
          v-if="hasTeamTasks"
          :open="activePanel === 'tasks'"
          @toggle="toggleTasks()"
        />
      </div>
    </ScrollArea>
  </aside>
</template>
