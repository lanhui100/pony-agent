<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { formatElapsedSeconds, type AgentRuntimeStatus } from "@/lib/runtime/status";
import { Loader2, Brain, Wrench, ShieldAlert, Sparkles } from "lucide-vue-next";

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
  return formatElapsedSeconds(elapsed);
});

const iconComponent = computed(() => {
  switch (props.status.mode) {
    case "thinking":
      return Brain;
    case "executing_tool":
      return Wrench;
    case "waiting_approval":
      return ShieldAlert;
    case "connecting":
      return Loader2;
    case "responding":
    default:
      return Sparkles;
  }
});
</script>

<template>
  <div
    class="flex items-center gap-1.5 py-1 text-[12px] text-stone-400 w-fit select-none"
    data-testid="agent-runtime-status-bar"
  >
    <Transition name="status-transition" mode="out-in">
      <div :key="props.status.mode + '-' + props.status.label" class="flex items-center gap-1.5">
        <component
          :is="iconComponent"
          :class="[
            'h-3.5 w-3.5 shrink-0',
            props.status.mode === 'connecting' ? 'animate-spin text-stone-400' : 'text-stone-400'
          ]"
          data-testid="agent-runtime-status-icon"
        />
        <span class="runtime-status-shimmer font-normal" data-testid="agent-runtime-status-label">
          {{ props.status.label }}
        </span>
        <span
          v-if="elapsedText"
          class="text-[11px] text-stone-400 tabular-nums"
          data-testid="agent-runtime-status-elapsed"
        >
          {{ elapsedText }}
        </span>
      </div>
    </Transition>
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

.status-transition-enter-active,
.status-transition-leave-active {
  transition: opacity 240ms cubic-bezier(0.2, 0, 0.2, 1), transform 240ms cubic-bezier(0.2, 0, 0.2, 1);
}

.status-transition-enter-from {
  opacity: 0;
  transform: translateY(2px);
}

.status-transition-leave-to {
  opacity: 0;
  transform: translateY(-2px);
}
</style>
