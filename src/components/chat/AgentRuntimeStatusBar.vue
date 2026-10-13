<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { formatElapsedSeconds, type AgentRuntimeStatus } from "@/lib/runtime/status";
import {
  Brain,
  ClipboardList,
  Compass,
  FileSearch,
  FileText,
  Globe,
  List,
  MessageSquareMore,
  Pen,
  PenLine,
  Plug,
  RefreshCw,
  ScanSearch,
  Search,
  ShieldAlert,
  Sparkles,
  Terminal,
  Wrench,
} from "lucide-vue-next";

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

const toolIconMap: Record<string, any> = {
  Run: Terminal,
  Ask: MessageSquareMore,
  Read: FileText,
  Search: Search,
  List: List,
  Glob: FileSearch,
  WebFetch: Globe,
  WebSearch: ScanSearch,
  MCPResource: Plug,
  ToolSearch: Wrench,
  Write: Pen,
  Edit: PenLine,
  Plan: ClipboardList,
};

function resolveToolIcon(canonicalToolName?: string | null, toolName?: string | null) {
  if (canonicalToolName && toolIconMap[canonicalToolName]) {
    return toolIconMap[canonicalToolName];
  }
  const normalized = (toolName || "").toLowerCase();
  if (normalized.includes("bash") || normalized.includes("sh") || normalized.includes("command")) return Terminal;
  if (normalized.includes("read") || normalized.includes("file")) return FileText;
  if (normalized.includes("write") || normalized.includes("edit")) return PenLine;
  if (normalized.includes("search") || normalized.includes("find")) return Search;
  if (normalized.includes("web") || normalized.includes("fetch")) return Globe;
  return Wrench;
}

const iconComponent = computed(() => {
  switch (props.status.mode) {
    case "retrying":
      return RefreshCw;
    case "thinking":
      return Brain;
    case "tool_drafting":
    case "executing_tool":
      return resolveToolIcon(props.status.canonicalToolName, props.status.toolName);
    case "waiting_approval":
      return ShieldAlert;
    case "connecting":
      return Compass;
    case "responding":
    default:
      return Sparkles;
  }
});

const iconAnimationClass = computed(() => {
  switch (props.status.mode) {
    case "retrying":
      return "animate-spin text-amber-500 dark:text-amber-400";
    case "connecting":
      return "animate-spin text-stone-400";
    case "thinking":
      return "runtime-icon-pulse text-stone-400";
    case "tool_drafting":
      return "runtime-icon-wiggle text-stone-400";
    case "executing_tool":
      return "runtime-icon-pulse text-stone-400";
    case "waiting_approval":
      return "runtime-icon-alert text-amber-500 dark:text-amber-400";
    case "responding":
    default:
      return "text-stone-400";
  }
});
</script>

<template>
  <div
    class="flex items-center gap-1.5 py-1 text-[12px] text-stone-400 w-fit select-none min-h-[24px]"
    data-testid="agent-runtime-status-bar"
    role="status"
    aria-live="polite"
    aria-atomic="true"
  >
    <div class="flex items-center gap-1.5">
      <component
        :is="iconComponent"
        :class="[
          'h-3.5 w-3.5 shrink-0 transition-colors duration-200',
          iconAnimationClass
        ]"
        data-testid="agent-runtime-status-icon"
      />
      <span class="runtime-status-shimmer font-normal" data-testid="agent-runtime-status-label">
        {{ displayLabel }}
      </span>
      <span
        v-if="elapsedText"
        class="text-[11px] text-stone-400 tabular-nums ml-0.5"
        data-testid="agent-runtime-status-elapsed"
      >
        {{ elapsedText }}
      </span>
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

/* 柔和呼吸脉冲动效 */
.runtime-icon-pulse {
  animation: icon-pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;
}

@keyframes icon-pulse {
  0%, 100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.6;
    transform: scale(0.94);
  }
}

/* 工具生成微摆动动效 */
.runtime-icon-wiggle {
  animation: icon-wiggle 1.8s ease-in-out infinite;
  transform-origin: center;
}

@keyframes icon-wiggle {
  0%, 100% {
    transform: rotate(0deg);
  }
  25% {
    transform: rotate(-10deg);
  }
  75% {
    transform: rotate(10deg);
  }
}

/* 审批提醒闪烁动效 */
.runtime-icon-alert {
  animation: icon-alert 1.6s ease-in-out infinite;
}

@keyframes icon-alert {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.45;
  }
}

@media (prefers-reduced-motion: reduce) {
  .runtime-status-shimmer {
    background: none;
    -webkit-text-fill-color: #78716c;
    color: #78716c;
    animation: none;
  }
  .runtime-icon-pulse,
  .runtime-icon-wiggle,
  .runtime-icon-alert {
    animation: none;
  }
}
</style>
