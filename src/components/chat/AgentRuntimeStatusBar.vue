<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { AlertCircle, CheckCircle2, Pause } from "lucide-vue-next";
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
  clearLabelMotionTimers();
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

// C-9：info 为中性/成功反馈（斜杠命令提示），需与 error 区分语义色，且不做 shimmer 动效。
const isInfoMode = computed(() => displayedStatus.value.mode === "info");

// wave-64 §2 F1：stopped 为「停止终态」（非进行中）：lucide Pause + 中性 stone 语义色，
// 禁止 emerald/绿系（非本项目色彩体系）、禁止 shimmer 扫光、禁止耗时计时。
const isStoppedMode = computed(() => displayedStatus.value.mode === "stopped");

const labelToneClass = computed(() => {
  if (displayedStatus.value.mode === "error") {
    return "text-rose-600 dark:text-rose-400 not-italic";
  }
  if (isInfoMode.value) {
    return "text-emerald-600 dark:text-emerald-400 not-italic";
  }
  if (isStoppedMode.value) {
    return "text-stone-600 dark:text-stone-400 not-italic";
  }
  return "runtime-status-shimmer text-stone-500 dark:text-stone-300 italic";
});

// ─── wave-pds-3 §3.4：状态行交接（原位替换 + rise/fall） ───────────────────
// 约束（契约 v5 R-B1）：
// - label 节点**不得重建**（既有 wave-11.2 F3 语义，本波继续锁住）。
// - rise 必须**真正生效**：`.runtime-status-shimmer` 与交接动效不能各写一条 `animation` 抢同一个简写
//   （scoped 规则的 `(0,2,0)` 会恒定压过 primitives 的 `(0,1,0)`，rise 静默失效）。
//   ⇒ 交接时把 `shimmer-sweep` 与 `runtime-status-label-rise` **并列在同一条 `animation` 声明里**。
// - 两个相位（`enter` / `enter-b`）交替出现，让这条声明真的变化一次，动画才会重播（免定时器）。
//   ⚠️ `enter-b` 是**刻意的第三枚举值**（不是笔误、不是非法值）：它存在的唯一目的就是让
//   `animation` 声明真的变化一次以触发重播。删掉它 = rise 只在首次交接时播放（R-B1 回归）。
// - 退场 fall：旧文案以绝对定位的幽灵节点在同一位置淡出下坠 180ms，纯视觉层，`aria-hidden`。
const LABEL_FALL_MS = 180;
/** 交接相位：`enter` / `enter-b` 交替，驱动「shimmer + rise」并列声明重播。
 *  `enter-b` 是**刻意的第三枚举值**（非笔误）：两相位交替才能让 `animation` 简写真正变化一次，
 *  从而重播 rise。若被当成非法值清理掉，rise 会退化为"只在首次交接时播放"。 */
const labelHandoffPhase = ref<"enter" | "enter-b" | null>(null);
/** 正在退场的旧文案（空 ⇒ 无幽灵节点）。 */
const fadingLabel = ref("");
let labelHandoffTick = 0;
let labelFadeTimer: ReturnType<typeof setTimeout> | null = null;

function clearLabelMotionTimers() {
  if (labelFadeTimer) {
    clearTimeout(labelFadeTimer);
    labelFadeTimer = null;
  }
}

watch(displayLabel, (nextLabel, previousLabel) => {
  if (nextLabel === previousLabel) {
    return;
  }

  labelHandoffTick += 1;
  labelHandoffPhase.value = labelHandoffTick % 2 === 1 ? "enter" : "enter-b";

  // fall：旧文案下坠淡出。
  if (previousLabel) {
    fadingLabel.value = previousLabel;
    if (labelFadeTimer) {
      clearTimeout(labelFadeTimer);
    }
    labelFadeTimer = setTimeout(() => {
      fadingLabel.value = "";
      labelFadeTimer = null;
    }, LABEL_FALL_MS);
  }
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
      <CheckCircle2
        v-if="isInfoMode"
        class="h-3.5 w-3.5 shrink-0 mr-1.5 text-emerald-500 dark:text-emerald-400"
        data-testid="agent-runtime-status-icon"
        :data-mode="displayedStatus.mode"
      />
      <AlertCircle
        v-else-if="displayedStatus.mode === 'error'"
        class="h-3.5 w-3.5 shrink-0 mr-1.5 text-rose-500"
        data-testid="agent-runtime-status-icon"
        :data-mode="displayedStatus.mode"
      />
      <!-- wave-64 §2 F1：停止终态用语义化的暂停图标（中性 stone），不得复用 info 的 CheckCircle2。
           装饰性图标对读屏隐藏，语义由 label 文本「已停止」承载（与同组件轨道 span 的 aria 口径一致）。 -->
      <Pause
        v-else-if="isStoppedMode"
        class="h-3.5 w-3.5 shrink-0 mr-1.5 text-stone-500 dark:text-stone-400"
        data-testid="agent-runtime-status-icon"
        :data-mode="displayedStatus.mode"
        aria-hidden="true"
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
      <div
        :class="labelToneClass"
        class="relative inline-flex min-w-0 max-w-full items-center font-normal"
      >
        <!-- wave-pds-3 §3.4 fall：旧文案在同一位置淡出下坠，纯视觉层（不参与读屏）。 -->
        <span
          v-if="fadingLabel"
          class="runtime-status-label-fall pointer-events-none absolute top-0 left-0 min-w-0 truncate font-normal"
          aria-hidden="true"
        >{{ fadingLabel }}</span>
        <span
          :class="labelToneClass"
          class="min-w-0 truncate font-normal"
          data-testid="agent-runtime-status-label"
          :data-pds-status="labelHandoffPhase ?? undefined"
        >{{ displayLabel }}</span>
        <span
          v-if="elapsedText && displayedStatus.mode !== 'error' && !isStoppedMode"
          class="shrink-0 text-[12px] italic font-normal tabular-nums"
          data-testid="agent-runtime-status-elapsed"
        >{{ elapsedText }}</span>
      </div>
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
    rgba(120, 113, 108, 0.45) 30%,
    rgba(28, 25, 23, 1) 50%,
    rgba(120, 113, 108, 0.45) 70%,
    rgba(120, 113, 108, 0.45) 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: shimmer-sweep 2.4s cubic-bezier(0.4, 0, 0.2, 1) infinite;
}

@keyframes shimmer-sweep {
  0% {
    background-position: 100% 0;
  }

  100% {
    background-position: -100% 0;
  }
}

:global(.dark) .runtime-status-shimmer {
  background: linear-gradient(
    90deg,
    rgba(168, 162, 158, 0.45) 0%,
    rgba(168, 162, 158, 0.45) 30%,
    rgba(255, 255, 255, 1) 50%,
    rgba(168, 162, 158, 0.45) 70%,
    rgba(168, 162, 158, 0.45) 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

/* wave-pds-3 §3.4 / R-B1：状态行交接。
   rise **不许**再靠「另一条规则写 `animation`」去和 `.runtime-status-shimmer` 抢同一个简写
   （scoped `(0,2,0)` 恒定压过 primitives `(0,1,0)` ⇒ rise 静默失效）。做法：
   把 `shimmer-sweep` 与 `runtime-status-label-rise` **并列在同一条 `animation` 声明**里；
   两个相位（`enter` / `enter-b`）交替下发，声明因此真的变化一次 ⇒ 动画重播（无需定时器）。
   fall 由本组件自带（180ms），只作用于「旧文案幽灵节点」，label 本体始终原位、不重建。 */
.runtime-status-shimmer[data-pds-status="enter"] {
  animation:
    shimmer-sweep 2.4s cubic-bezier(0.4, 0, 0.2, 1) infinite,
    runtime-status-label-rise 240ms cubic-bezier(0.3, 0.7, 0.4, 1) both;
}

.runtime-status-shimmer[data-pds-status="enter-b"] {
  animation:
    shimmer-sweep 2.4s cubic-bezier(0.4, 0, 0.2, 1) infinite,
    runtime-status-label-rise-b 240ms cubic-bezier(0.3, 0.7, 0.4, 1) both;
}

/* error / info 语义下 label 不挂 shimmer（没有扫光），交接只用 rise 单动画。
   注：`enter-b` 是双相位重播用的**刻意第三枚举值**（与 `enter` 同构、只是名字不同），
   不得按"非法值"删除；两个相位交替下发才能让 `animation` 简写变化并触发重播。 */
[data-pds-status="enter"]:not(.runtime-status-shimmer) {
  animation: runtime-status-label-rise 240ms cubic-bezier(0.3, 0.7, 0.4, 1) both;
}

[data-pds-status="enter-b"]:not(.runtime-status-shimmer) {
  animation: runtime-status-label-rise-b 240ms cubic-bezier(0.3, 0.7, 0.4, 1) both;
}

@keyframes runtime-status-label-rise {
  from {
    opacity: 0;
    transform: translateY(6px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes runtime-status-label-rise-b {
  from {
    opacity: 0;
    transform: translateY(6px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.runtime-status-label-fall {
  animation: runtime-status-label-fall 180ms cubic-bezier(0, 0, 0.2, 1) both;
}

@keyframes runtime-status-label-fall {
  from {
    opacity: 1;
    transform: translateY(0);
  }

  to {
    opacity: 0;
    transform: translateY(-4px);
  }
}

@media (prefers-reduced-motion: reduce) {
  /* 降级：退场幽灵直接不渲染（动画被关掉后它会与 label 叠字）。 */
  .runtime-status-label-fall {
    display: none !important;
    animation: none !important;
  }

  .runtime-status-orbit,
  .runtime-status-orbit * {
    animation: none !important;
  }

  .runtime-status-orbit *::after {
    animation: none !important;
  }

  .runtime-status-shimmer {
    /* R-B7：降级时把「透明填充 + 扫光渐变」复位成文字自身的颜色（`currentColor`），
       由 tone class（text-stone-500 / dark:text-stone-300）决定最终色值。
       禁止硬编码灰：`#78716c` 在 dark + reduced-motion 下实测 3.6:1 < AA 4.5:1。 */
    background: none;
    -webkit-text-fill-color: currentColor;
    animation: none;
  }

  /* 交接相位把 shimmer 与 rise 并列在同一条 `animation` 里，特异度高于上面的单类规则，
     故降级时必须显式按同一批选择器关掉（否则降级下 rise 仍会播）。 */
  .runtime-status-shimmer[data-pds-status="enter"],
  .runtime-status-shimmer[data-pds-status="enter-b"],
  [data-pds-status="enter"]:not(.runtime-status-shimmer),
  [data-pds-status="enter-b"]:not(.runtime-status-shimmer) {
    animation: none !important;
  }
}
</style>
