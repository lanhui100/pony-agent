<script setup lang="ts">
import { computed } from "vue";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Check,
  Copy,
  Gauge,
  MessageSquareMore,
  Orbit,
  ScanSearch,
  Wrench,
  Zap
} from "lucide-vue-next";
import Tooltip from "@/components/ui/Tooltip.vue";
import { isDebugLoggingEnabled } from "@/lib/runtime/utils";
import { useRuntimeStore } from "@/stores/runtime";

const props = defineProps<{
  sessionId: string;
  turnCount: number;
  modelCallCount: number;
  toolCallCount: number;
  copied: boolean;
  /** wave-54 C4：上下文占用（末次调用 contextInputTokens，回退 S1）；无数据为 null ⇒ 显示 `—`。 */
  inputTokensTotal: number | null;
  outputTokensTotal: number | null;
  cacheHitTokensTotal: number | null;
  cacheHitRatio: string;
  tokenGenerationSpeed: number | null;
  tokenSpeedCalculationMode?: string;
  speedDebugInfo?: {
    mode?: string;
    /** wave-54 C2/C6：`turn_weighted` | `call_only` | `none`。 */
    basis?: string;
    /** 参与求和的已完成调用数。 */
    calls?: number;
    speed?: number | null;
    inputTokens?: number | null;
    outputTokens?: number | null;
    durationMs?: number | null;
    genMs?: number | null;
    ttftMs?: number | null;
    contextWindow?: number | null;
    ratio?: string | null;
    /** wave-54 C3：`estimated_chars` ⇒ 数字带 `≈` 前缀。 */
    origin?: string | null;
  };
  showContextUsage: boolean;
  contextDisplayTokens: number | null;
  contextWindowTokens: number | null;
}>();

const emit = defineEmits<{ copySessionId: [] }>();

function resolveRuntimeStoreSafe() {
  try {
    return useRuntimeStore();
  } catch {
    return null;
  }
}

const blockDetail = computed(() => resolveRuntimeStoreSafe()?.governanceBlockDetail ?? null);

const debugAttrsEnabled = computed(() => isDebugLoggingEnabled());

/** wave-54 C6：速度位基准（`turn_weighted` = 加权 N 次调用；`call_only` = 当前调用）。 */
const speedBasis = computed(() => props.speedDebugInfo?.basis ?? props.tokenSpeedCalculationMode ?? "none");

/** wave-54 C3：估算来源必须显式降级（`≈`），不得冒充真值。 */
const isEstimatedUsage = computed(() => props.speedDebugInfo?.origin === "estimated_chars");

const speedTooltip = computed(() => {
  const calls = props.speedDebugInfo?.calls;
  if (speedBasis.value === "call_only") {
    return "当前调用的生成速度（单次调用，含生成耗时口径）";
  }
  if (speedBasis.value === "turn_weighted" && typeof calls === "number" && calls > 0) {
    return `加权生成速度（${calls} 次调用，Σoutput/Σ生成耗时）`;
  }
  return "生成速度（加权，暂无已完成调用）";
});

/** C6 `data-speed-debug`：`basis=…;calls=…;in=…;out=…;dur=…;ttft=…;gen=…;win=…;ratio=…;origin=…`。 */
const speedDebugString = computed(() => {
  const info = props.speedDebugInfo;
  const inVal = info?.inputTokens ?? props.inputTokensTotal ?? "-";
  const outVal = info?.outputTokens ?? props.outputTokensTotal ?? "-";
  const durVal = info?.durationMs ?? "-";
  const ttftVal = info?.ttftMs ?? "-";
  const genVal = info?.genMs ?? "-";
  const winVal = info?.contextWindow ?? props.contextWindowTokens ?? "-";
  let ratioVal = info?.ratio ?? "-";
  if (ratioVal === "-" && props.contextWindowTokens && props.contextWindowTokens > 0 && inVal !== "-") {
    ratioVal = (Number(inVal) / props.contextWindowTokens).toFixed(3);
  }
  return [
    `basis=${speedBasis.value}`,
    `calls=${info?.calls ?? 0}`,
    `in=${inVal}`,
    `out=${outVal}`,
    `dur=${durVal}`,
    `ttft=${ttftVal}`,
    `gen=${genVal}`,
    `win=${winVal}`,
    `ratio=${ratioVal}`,
    `origin=${info?.origin ?? "-"}`
  ].join(";");
});

/**
 * wave-54 C6 紧凑格式：`<1000` 原值；`<1e6` 一位小数 `1.9K`；`≥1e6` 一位小数 `1.2M`。
 * 无数据（null / 非有限值）⇒ `—`（禁止渲染 `0`）。
 */
function formatCompactInteger(value?: number | null) {
  if (value == null || !Number.isFinite(value)) { return "—"; }
  if (value >= 1_000_000) { return `${(value / 1_000_000).toFixed(1)}M`; }
  if (value >= 1_000) { return `${(value / 1_000).toFixed(1)}K`; }
  return String(value);
}

/** 速度显示：无数据 `—`；估算来源加 `≈` 前缀（C3）。 */
function formatTokenSpeed(value?: number | null) {
  if (value == null || !Number.isFinite(value)) { return "—"; }
  const text = `${Math.round(value)} t/s`;
  return isEstimatedUsage.value ? `≈${text}` : text;
}

/**
 * wave-54 C3：估算标记覆盖 token 三位（输入/输出/缓存），不只速度位。
 * 无数据仍是 `—`（不渲染 `≈—`）。
 */
function formatTokenValue(value?: number | null) {
  const text = formatCompactInteger(value);
  if (text === "—") { return text; }
  return isEstimatedUsage.value ? `≈${text}` : text;
}
</script>

<template>
  <section class="pb-16" data-open="true">
    <div class="flex w-full items-center justify-between gap-2 text-left" data-testid="status-panel-toggle">
      <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
        <Tooltip text="状态概览">
          <ScanSearch class="h-3.5 w-3.5" />
        </Tooltip>
        <span>状态</span>
      </div>
      <div class="flex items-center gap-x-3 text-[11px] leading-5 text-stone-600">
        <Tooltip :text="`复制 Session ID: ${sessionId}`">
          <button
            class="inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
            type="button"
            data-testid="status-copy-session-id"
            @click.stop="emit('copySessionId')"
          >
            <component :is="copied ? Check : Copy" class="h-3 w-3" />
          </button>
        </Tooltip>
        <!-- Wave-17 C2：计数位数字原地变化（9 → 12 → 345）会横向推移相邻位点。
             修法：承载 span 自身 `tabular-nums` + 固定最小宽度预留 4 位数字空间，
             位数变化只消耗预留宽度，不移动图标与相邻位点。 -->
        <Tooltip text="对话轮次数">
          <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-turn-count">
            <MessageSquareMore class="h-3 w-3 text-stone-400" />
            <span class="inline-block min-w-[2.25rem] text-right">{{ turnCount }}</span>
          </span>
        </Tooltip>
        <Tooltip text="模型调用次数">
          <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-model-call-count">
            <Orbit class="h-3 w-3 text-stone-400" />
            <span class="inline-block min-w-[2.25rem] text-right">{{ modelCallCount }}</span>
          </span>
        </Tooltip>
        <Tooltip text="工具调用次数">
          <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-tool-call-count">
            <Wrench class="h-3 w-3 text-stone-400" />
            <span class="inline-block min-w-[2.25rem] text-right">{{ toolCallCount }}</span>
          </span>
        </Tooltip>
      </div>
    </div>

    <section class="border-t border-stone-200/60 pt-3 mt-3">
      <div class="rounded-[0.45rem] bg-[#f6f0e8] px-3 py-2">
        <!-- Token metrics -->
        <div class="flex items-center justify-around text-[11px] leading-5 text-stone-600">
          <!-- wave-54 C4/C6：「输入」口径 = 末次调用 contextInputTokens（回退 S1），
               无数据一律 `—`（禁止渲染 0）；估算来源加 `≈`。非会话累计。 -->
          <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-input-tokens">
            <Tooltip text="输入">
              <ArrowUp class="h-3 w-3 text-stone-400" />
            </Tooltip>
            <Tooltip text="上下文输入（最近一次模型调用，协议归一口径）">
              <span class="inline-block min-w-[2.25rem] text-right">{{ formatTokenValue(inputTokensTotal) }}</span>
            </Tooltip>
          </span>
          <Tooltip text="输出">
            <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-output-tokens">
              <ArrowDown class="h-3 w-3 text-stone-400" />
              <span class="inline-block min-w-[2.25rem] text-right">{{ formatTokenValue(outputTokensTotal) }}</span>
            </span>
          </Tooltip>
          <Tooltip text="缓存读取">
            <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-cache-tokens">
              <Zap class="h-3 w-3 text-stone-400" />
              <span class="inline-block min-w-[2.25rem] text-right">{{ formatTokenValue(cacheHitTokensTotal) }}</span>
              <span v-if="cacheHitRatio" class="inline-block min-w-[2.25rem] text-right text-stone-400">· {{ cacheHitRatio }}</span>
            </span>
          </Tooltip>
          <!-- 速度位在无数据时渲染 "—"（无数字文本节点），故预留宽度落在承载 span 自身，
               使其在 9 / 12 / 1234 三档（含注入探针）下宽度恒定。 -->
          <span
            class="inline-flex min-w-[4rem] items-center gap-1 tabular-nums"
            data-testid="status-metric-token-speed"
            :data-speed-mode="tokenSpeedCalculationMode || 'none'"
            :data-speed-basis="speedBasis"
            :data-speed-debug="debugAttrsEnabled ? speedDebugString : undefined"
          >
            <Tooltip text="生成速度">
              <Gauge class="h-3 w-3 text-stone-400" />
            </Tooltip>
            <Tooltip :text="speedTooltip">
              <span>{{ formatTokenSpeed(tokenGenerationSpeed) }}</span>
            </Tooltip>
          </span>
        </div>

        <!-- Wave-7 G2：拦截态横幅 UI（触及硬限额 opt-in 拦截） -->
        <div
          v-if="blockDetail"
          class="mt-2 rounded-[0.4rem] border border-rose-200 bg-rose-50/80 p-2 text-[11px] text-rose-800 space-y-1"
          data-testid="governance-block-banner"
        >
          <div class="flex items-center gap-1.5 font-medium text-rose-900">
            <AlertTriangle class="h-3.5 w-3.5 text-rose-600 shrink-0" />
            <span>已触发用量治理拦截</span>
          </div>
          <div class="leading-relaxed">
            当前消耗 {{ blockDetail.currentText }} / 硬限额 {{ blockDetail.limitText }}
          </div>
          <div class="flex items-center gap-2 pt-0.5">
            <button
              type="button"
              class="text-rose-700 underline hover:text-rose-900"
              data-testid="governance-override-btn"
              @click="(resolveRuntimeStoreSafe() as any)?.overrideGovernanceBlock?.()"
            >
              本次解除拦截
            </button>
          </div>
        </div>
      </div>
    </section>
  </section>
</template>
