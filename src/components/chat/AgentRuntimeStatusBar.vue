<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { formatElapsedSeconds, type AgentRuntimeStatus } from "@/lib/runtime/status";
import { Loader2 } from "lucide-vue-next";

interface Props {
  status: AgentRuntimeStatus;
}

const props = defineProps<Props>();

const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | null = null;

onMounted(() => {
  timer = setInterval(() => {
    now.value = Date.now();
  }, 1000);
});

onUnmounted(() => {
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
});

const elapsedText = computed(() => {
  if (!props.status.startTime) return null;
  const elapsed = Math.max(0, now.value - props.status.startTime);
  return `，用时 ${formatElapsedSeconds(elapsed)} ···`;
});

const displayLabel = computed(() => {
  const raw = props.status.label || "";
  // 当有耗时显示时，剥离末尾的省略号避免“思考中...，用时 5秒 ···”重复省略号
  return elapsedText.value ? raw.replace(/[…\.\s]+$/, "") : raw;
});
</script>

<template>
  <div
    class="flex items-center gap-1.5 py-1 text-[12px] text-stone-400 w-fit select-none min-h-[24px] italic"
    data-testid="agent-runtime-status-bar"
    role="status"
    aria-live="polite"
    aria-atomic="true"
  >
    <div class="flex items-center">
      <Loader2
        class="h-3.5 w-3.5 shrink-0 mr-1.5 animate-spin text-stone-400 transition-colors duration-200"
        data-testid="agent-runtime-status-icon"
      />
      <span class="runtime-status-shimmer font-normal" data-testid="agent-runtime-status-label">{{ displayLabel }}</span><span
        v-if="elapsedText"
        class="text-[11px] text-stone-400 tabular-nums"
        data-testid="agent-runtime-status-elapsed"
      >{{ elapsedText }}</span>
    </div>
  </div>
</template>

<style scoped>
.runtime-status-shimmer {
  background: linear-gradient(
    90deg,
    rgba(120, 113, 108, 0.7) 0%,
    rgba(28, 25, 23, 0.95) 50%,
    rgba(120, 113, 108, 0.7) 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: shimmer-sweep 2.4s ease-in-out infinite;
}

@keyframes shimmer-sweep {
  0% {
    background-position: 200% 0;
  }
  100% {
    background-position: -200% 0;
  }
}

:global(.dark) .runtime-status-shimmer {
  background: linear-gradient(
    90deg,
    rgba(168, 162, 158, 0.7) 0%,
    rgba(245, 245, 244, 0.98) 50%,
    rgba(168, 162, 158, 0.7) 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

@media (prefers-reduced-motion: reduce) {
  .runtime-status-shimmer {
    background: none;
    -webkit-text-fill-color: #78716c;
    color: #78716c;
    animation: none;
  }
}
</style>

