<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import HomeTracePanel from "@/components/HomeTracePanel.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { clearTraceProjectionMemo } from "@/lib/runtime/trace-projection";
import {
  canonicalTraceTimelineKind,
  projectedTurnTimeline,
  providerReturnedCacheHitInputTokens,
  useCopyFeedback,
  useTraceProjection
} from "@/lib/runtime/useTraceProjection";

/**
 * PA-096：观测页 Trace tab 的自包含宿主。
 *
 * - open 初始 false、onMounted 后下一 tick 置 true：
 *   ①挂载瞬间冻结守卫生效（liveTurnEnabled=false，PA-086 语义保留）；
 *   ②触发 HomeTracePanel 内部 open false→true watch，执行首次滚动到底部
 *   （用户落点 = 最新 turn，而非 timeline 顶部）。
 * - sessionId 切换时清理 projection memo 与复制反馈；卸载仅清理 copy timer
 *   （全局 memo 清理由 home 页 HomeSidebar 承担；out-in Transition 保证二者不共存）。
 */

const runtimeStore = useRuntimeStore();

const open = ref(false);
const { copiedKey, copyText, resetCopy, dispose } = useCopyFeedback();

const { orderedTurnTraces } = useTraceProjection({ liveTurnEnabled: open });

onMounted(() => {
  void nextTick().then(() => {
    open.value = true;
  });
});

watch(
  () => runtimeStore.sessionId,
  () => {
    resetCopy();
    clearTraceProjectionMemo();
  }
);

onBeforeUnmount(() => {
  dispose();
});
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col" data-testid="trace-inspector">
    <HomeTracePanel
      :turns="orderedTurnTraces"
      :session-id="runtimeStore.sessionId"
      :copied-key="copiedKey"
      :open="open"
      :expanded="true"
      :canonical-kind="canonicalTraceTimelineKind"
      :turn-timeline="projectedTurnTimeline"
      :provider-returned-cache-hit-input-tokens="providerReturnedCacheHitInputTokens"
      @copy="copyText"
      @toggle="open = !open"
    />
  </section>
</template>
