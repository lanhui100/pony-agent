<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { formatElapsedSeconds, type AgentRuntimeStatus } from "@/lib/runtime/status";

interface Props {
  status: AgentRuntimeStatus;
}

const props = defineProps<Props>();

const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | null = null;

// 状态去瞬态闪烁与平滑停留机制：
// 当进入工具执行、工具草稿、重试等具象状态时，至少保持展示 600ms，
// 避免因为底层事件瞬时触发又跳回“思考中/处理中”导致用户根本看不清
const displayedStatus = ref<AgentRuntimeStatus>(props.status);
let statusHoldTimer: ReturnType<typeof setTimeout> | null = null;
let lastImportantStatusTime = 0;
const MIN_STATUS_HOLD_MS = 600;

function isHighPriorityMode(mode: AgentRuntimeStatus["mode"]) {
  return mode === "executing_tool" || mode === "tool_drafting" || mode === "retrying" || mode === "waiting_approval";
}

watch(
  () => props.status,
  (nextStatus) => {
    const current = displayedStatus.value;
    const nowMs = Date.now();

    // 如果当前正在展示高优先级（如工具调用），但新状态降级为通用状态（如思考中），
    // 检查是否满足最小停留时间要求，如果不满足则延迟切换
    if (isHighPriorityMode(current.mode) && !isHighPriorityMode(nextStatus.mode)) {
      const elapsed = nowMs - lastImportantStatusTime;
      if (elapsed < MIN_STATUS_HOLD_MS) {
        if (statusHoldTimer) clearTimeout(statusHoldTimer);
        statusHoldTimer = setTimeout(() => {
          displayedStatus.value = props.status;
          statusHoldTimer = null;
        }, MIN_STATUS_HOLD_MS - elapsed);
        return;
      }
    }

    if (statusHoldTimer) {
      clearTimeout(statusHoldTimer);
      statusHoldTimer = null;
    }

    if (isHighPriorityMode(nextStatus.mode)) {
      lastImportantStatusTime = nowMs;
    }

    displayedStatus.value = nextStatus;
  },
  { deep: true }
);

onMounted(() => {
  if (isHighPriorityMode(props.status.mode)) {
    lastImportantStatusTime = Date.now();
  }
  timer = setInterval(() => {
    now.value = Date.now();
  }, 1000);
});

onUnmounted(() => {
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
  if (statusHoldTimer) {
    clearTimeout(statusHoldTimer);
    statusHoldTimer = null;
  }
});

const elapsedText = computed(() => {
  const startTime = displayedStatus.value.startTime;
  if (!startTime) return null;
  const elapsed = Math.max(0, now.value - startTime);
  return `，用时 ${formatElapsedSeconds(elapsed)} ···`;
});

const displayLabel = computed(() => {
  const raw = displayedStatus.value.label || "";
  // 当有耗时显示时，剥离末尾的省略号避免“思考中...，用时 5秒 ···”重复省略号
  return elapsedText.value ? raw.replace(/[…\.\s]+$/, "") : raw;
});
</script>

<template>
  <div
    class="flex items-center gap-1.5 py-1 text-[12px] text-stone-400 w-fit select-none min-h-[24px] italic font-normal"
    data-testid="agent-runtime-status-bar"
    role="status"
    aria-live="polite"
    aria-atomic="true"
  >
    <div class="flex items-center">
      <!-- 贪吃蛇式的连续旋转动效图标 -->
      <svg
        class="h-3.5 w-3.5 shrink-0 mr-1.5 text-stone-400 snake-spinner"
        viewBox="0 0 16 16"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        data-testid="agent-runtime-status-icon"
      >
        <circle
          cx="8"
          cy="8"
          r="6"
          stroke="currentColor"
          stroke-width="1.75"
          stroke-linecap="round"
          class="snake-spinner-path"
        />
      </svg>
      <span class="runtime-status-shimmer text-stone-500 dark:text-stone-300 italic font-normal" data-testid="agent-runtime-status-label">{{ displayLabel }}</span><span
        v-if="elapsedText"
        class="runtime-status-shimmer text-[12px] text-stone-500 dark:text-stone-300 italic font-normal tabular-nums"
        data-testid="agent-runtime-status-elapsed"
      >{{ elapsedText }}</span>
    </div>
  </div>
</template>

<style scoped>
/* 贪吃蛇动效：环形路径伸缩与旋转结合 */
.snake-spinner {
  animation: snake-rotate 1.4s linear infinite;
}

.snake-spinner-path {
  stroke-dasharray: 28 38;
  stroke-dashoffset: 0;
  animation: snake-dash 1.4s ease-in-out infinite;
}

@keyframes snake-rotate {
  0% {
    transform: rotate(0deg);
  }
  100% {
    transform: rotate(360deg);
  }
}

@keyframes snake-dash {
  0% {
    stroke-dasharray: 4 34;
    stroke-dashoffset: 0;
  }
  50% {
    stroke-dasharray: 26 12;
    stroke-dashoffset: -10;
  }
  100% {
    stroke-dasharray: 4 34;
    stroke-dashoffset: -38;
  }
}

.runtime-status-shimmer {
  font-style: italic;
  font-synthesis: style;
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
  .snake-spinner {
    animation: none;
  }
  .snake-spinner-path {
    animation: none;
  }
}
</style>

