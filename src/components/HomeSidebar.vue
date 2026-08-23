<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import PlanPanel from "@/components/PlanPanel.vue";
import DebugPanel from "@/components/DebugPanel.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import { clearTraceProjectionMemo } from "@/lib/runtime/trace-projection";
import { useCopyFeedback, useTraceProjection } from "@/lib/runtime/useTraceProjection";

/**
 * PA-096：右侧栏瘦身为纯对话过程面板（状态 / 计划 / 调试）。
 *
 * - Tools 目录移至配置页（ConfigToolsSection）；Trace 移至遥测页（TraceInspector）。
 * - 会话级聚合由共享管线 useTraceProjection 供给；liveTurnEnabled 恒 false——
 *   与原"trace 面板折叠时聚合不含进行中 turn"的行为逐位一致。
 */

const runtimeStore = useRuntimeStore();

const {
  error,
  fallbackReason,
  isSubmitting,
  sessionError,
  sessionId,
  sessionOperation
} = storeToRefs(runtimeStore);

const activePanel = ref<"plan" | "debug" | "">("");
const { copiedKey, copyText, resetCopy, dispose } = useCopyFeedback();

// 状态面板聚合数据源（PA-096-R1 拆分线）：不消费活跃 turn（守卫恒关）。
const {
  contextDisplayTokens,
  currentContextWindowTokens,
  sessionCacheHitRatio,
  sessionCacheHitTokensTotal,
  sessionInputTokensTotal,
  sessionModelCallCount,
  sessionOutputTokensTotal,
  sessionToolCallCount,
  sessionTurnCount,
  showContextUsage
} = useTraceProjection({ liveTurnEnabled: ref(false) });

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

watch(sessionId, () => {
  resetCopy();
  clearTraceProjectionMemo();
});

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
          :show-context-usage="showContextUsage"
          :context-display-tokens="contextDisplayTokens"
          :context-window-tokens="currentContextWindowTokens"
          :status-summary="sessionStatusSummary"
          :fallback-reason="fallbackReason"
          :error="error"
          @copy-session-id="copyText('session-id', sessionId)"
        />

        <PlanPanel
          :session-id="sessionId"
          :open="activePanel === 'plan'"
          @toggle="togglePlan()"
        />

        <DebugPanel
          :active="activePanel === 'debug'"
          @toggle="activePanel = activePanel === 'debug' ? '' : 'debug'"
        />
      </div>
    </ScrollArea>
  </aside>
</template>
