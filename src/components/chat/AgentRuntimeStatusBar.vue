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
    class="flex items-center gap-2 px-3 py-1.5 text-[12px] text-stone-500 bg-stone-50/80 rounded-md border border-stone-200/60 w-fit select-none animate-pulse duration-1000"
    data-testid="agent-runtime-status-bar"
  >
    <component
      :is="iconComponent"
      :class="[
        'h-3.5 w-3.5 shrink-0',
        props.status.mode === 'connecting' ? 'animate-spin text-stone-400' : 'text-stone-500'
      ]"
      data-testid="agent-runtime-status-icon"
    />
    <span class="font-medium text-stone-700" data-testid="agent-runtime-status-label">
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
</template>
