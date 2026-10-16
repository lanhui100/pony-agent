<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { AlertCircle } from "lucide-vue-next";
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
    class="flex w-fit max-w-full min-w-0 items-center gap-1.5 whitespace-nowrap py-1 text-[12px] text-stone-400 select-none min-h-[24px] italic font-normal"
    data-testid="agent-runtime-status-bar"
    role="status"
    aria-live="polite"
    aria-atomic="true"
    :data-mode="displayedStatus.mode"
  >
    <div class="flex min-w-0 items-center">
      <AlertCircle
        v-if="displayedStatus.mode === 'error'"
        class="h-3.5 w-3.5 shrink-0 mr-1.5 text-rose-500"
        data-testid="agent-runtime-status-icon"
        :data-mode="displayedStatus.mode"
      />
      <!--
        Wave-11 契约 §2.9：纯 CSS「轨道 + 卫星点」微动效，禁止 JS 定时器/rAF 驱动。
        根节点 data-testid=agent-runtime-status-icon + 类 runtime-status-orbit，
        两个直接子节点分别为虚线轨道（慢速反向自转）与卫星点（绕心公转 + 呼吸缩放）。
        wave-11.2 F5：图标色提升为 stone-500 / dark:stone-300，环 1.5px 且不再降透明度，
        使虚线环对底色的 WCAG 对比度由不足 3:1 提升到实测 4.11:1（背板 #f2ede6）。
      -->
      <span
        v-else
        class="runtime-status-orbit relative inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center mr-1.5 text-stone-500 dark:text-stone-300"
        data-testid="agent-runtime-status-icon"
        :data-mode="displayedStatus.mode"
        aria-hidden="true"
      >
        <span
          class="runtime-status-orbit-ring"
          data-testid="agent-runtime-status-orbit-ring"
        />
        <span
          class="runtime-status-orbit-dot"
          data-testid="agent-runtime-status-orbit-dot"
        />
      </span>
      <span
        :class="displayedStatus.mode === 'error' ? 'text-rose-600 dark:text-rose-400 not-italic' : 'runtime-status-shimmer text-stone-500 dark:text-stone-300 italic'"
        class="min-w-0 truncate font-normal"
        data-testid="agent-runtime-status-label"
      >{{ displayLabel }}</span><span
        v-if="elapsedText && displayedStatus.mode !== 'error'"
        class="runtime-status-shimmer shrink-0 text-[12px] text-stone-500 dark:text-stone-300 italic font-normal tabular-nums"
        data-testid="agent-runtime-status-elapsed"
      >{{ elapsedText }}</span>
    </div>
  </div>
</template>

<style scoped>
/* ── 轨道 + 卫星点动效（纯 CSS keyframes） ─────────────────────── */
.runtime-status-orbit {
  overflow: visible;
}

.runtime-status-orbit-ring,
.runtime-status-orbit-dot {
  position: absolute;
  inset: 0;
  display: block;
  border-radius: 9999px;
}

.runtime-status-orbit-ring {
  /* wave-11.2 F5：1.5px 实心虚线（不再降 opacity），保证对底色对比度 ≥3:1 */
  border: 1.5px dashed currentColor;
  animation: runtime-status-orbit-ring-spin 1500ms linear infinite reverse;
}

.runtime-status-orbit-dot {
  animation: runtime-status-orbit-dot-orbit 1800ms linear infinite;
}

.runtime-status-orbit-dot::after {
  content: "";
  position: absolute;
  top: 0;
  left: 50%;
  width: 3px;
  height: 3px;
  margin-left: -1.5px;
  border-radius: 9999px;
  background: currentColor;
  animation: runtime-status-orbit-dot-breathe 1800ms ease-in-out infinite;
}

/* data-mode 驱动的节奏差异：思考/等待更慢，工具执行更快。
   wave-11.2 F6：ring 各 mode 收紧到 1200–1600ms（虚线漂移可辨），
   dot 保持 1200–2400ms；所有节点所有 mode 均落在 1200–2400ms 内。 */
.runtime-status-orbit[data-mode="connecting"] .runtime-status-orbit-ring {
  animation-duration: 1600ms;
}

.runtime-status-orbit[data-mode="connecting"] .runtime-status-orbit-dot {
  animation-duration: 2200ms;
}

.runtime-status-orbit[data-mode="thinking"] .runtime-status-orbit-ring {
  animation-duration: 1500ms;
}

.runtime-status-orbit[data-mode="thinking"] .runtime-status-orbit-dot {
  animation-duration: 2000ms;
}

.runtime-status-orbit[data-mode="responding"] .runtime-status-orbit-ring {
  animation-duration: 1400ms;
}

.runtime-status-orbit[data-mode="responding"] .runtime-status-orbit-dot {
  animation-duration: 1600ms;
}

.runtime-status-orbit[data-mode="tool_drafting"] .runtime-status-orbit-ring {
  animation-duration: 1300ms;
}

.runtime-status-orbit[data-mode="tool_drafting"] .runtime-status-orbit-dot {
  animation-duration: 1300ms;
}

.runtime-status-orbit[data-mode="executing_tool"] .runtime-status-orbit-ring {
  animation-duration: 1250ms;
}

.runtime-status-orbit[data-mode="executing_tool"] .runtime-status-orbit-dot {
  animation-duration: 1200ms;
}

.runtime-status-orbit[data-mode="waiting_approval"] .runtime-status-orbit-ring {
  animation-duration: 1600ms;
}

.runtime-status-orbit[data-mode="waiting_approval"] .runtime-status-orbit-dot {
  animation-duration: 2200ms;
}

.runtime-status-orbit[data-mode="retrying"] .runtime-status-orbit-ring {
  animation-duration: 1200ms;
}

.runtime-status-orbit[data-mode="retrying"] .runtime-status-orbit-dot {
  animation-duration: 1200ms;
}

@keyframes runtime-status-orbit-ring-spin {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}

@keyframes runtime-status-orbit-dot-orbit {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}

@keyframes runtime-status-orbit-dot-breathe {
  0%,
  100% {
    transform: scale(0.7);
    opacity: 0.55;
  }

  50% {
    transform: scale(1.15);
    opacity: 1;
  }
}

/* wave-11.2 F3：label 原位替换（不再 :key 重建 + 仅淡入），消除模式切换时的空白帧；
   持续动效由 .runtime-status-shimmer（文字扫光效果）承担。
   通过 text-fill-color: transparent + background-clip: text 让高亮光束横向匀速扫过文字。 */
.runtime-status-shimmer {
  font-style: italic;
  font-synthesis: style;
  background: linear-gradient(
    90deg,
    rgba(120, 113, 108, 0.45) 0%,
    rgba(120, 113, 108, 0.45) 35%,
    rgba(28, 25, 23, 1) 50%,
    rgba(120, 113, 108, 0.45) 65%,
    rgba(120, 113, 108, 0.45) 100%
  );
  background-size: 250% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: shimmer-sweep 2.2s ease-in-out infinite;
}

@keyframes shimmer-sweep {
  0% {
    background-position: 100% 0;
  }

  100% {
    background-position: -150% 0;
  }
}

:global(.dark) .runtime-status-shimmer {
  background: linear-gradient(
    90deg,
    rgba(168, 162, 158, 0.45) 0%,
    rgba(168, 162, 158, 0.45) 35%,
    rgba(255, 255, 255, 1) 50%,
    rgba(168, 162, 158, 0.45) 65%,
    rgba(168, 162, 158, 0.45) 100%
  );
  background-size: 250% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

@media (prefers-reduced-motion: reduce) {
  .runtime-status-orbit,
  .runtime-status-orbit * {
    animation: none !important;
  }

  .runtime-status-orbit *::after {
    animation: none !important;
  }

  .runtime-status-shimmer {
    background: none;
    -webkit-text-fill-color: #78716c;
    color: #78716c;
    animation: none;
  }
}
</style>
