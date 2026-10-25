<script setup lang="ts">
import { computed } from "vue";
import {
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

const props = defineProps<{
  sessionId: string;
  turnCount: number;
  modelCallCount: number;
  toolCallCount: number;
  copied: boolean;
  /** Wave-15 S1：当前上下文占用（最近一次模型调用 input_tokens），无 per-call 来源时为 null（显示 0）。 */
  inputTokensTotal: number | null;
  outputTokensTotal: number;
  cacheHitTokensTotal: number;
  cacheHitRatio: string;
  tokenGenerationSpeed: number | null;
  tokenSpeedCalculationMode?: string;
  speedDebugInfo?: {
    mode?: string;
    speed?: number | null;
    inputTokens?: number | null;
    outputTokens?: number | null;
    durationMs?: number | null;
    ttftMs?: number | null;
    contextWindow?: number | null;
    ratio?: string | null;
  };
  showContextUsage: boolean;
  contextDisplayTokens: number | null;
  contextWindowTokens: number | null;
}>();

const emit = defineEmits<{ copySessionId: [] }>();

const debugAttrsEnabled = computed(() => isDebugLoggingEnabled());

const speedDebugString = computed(() => {
  const info = props.speedDebugInfo;
  const inVal = info?.inputTokens ?? props.inputTokensTotal ?? "-";
  const outVal = info?.outputTokens ?? props.outputTokensTotal ?? "-";
  const durVal = info?.durationMs ?? "-";
  const ttftVal = info?.ttftMs ?? "-";
  const winVal = info?.contextWindow ?? props.contextWindowTokens ?? "-";
  let ratioVal = info?.ratio ?? "-";
  if (ratioVal === "-" && props.contextWindowTokens && props.contextWindowTokens > 0 && inVal !== "-") {
    ratioVal = (Number(inVal) / props.contextWindowTokens).toFixed(3);
  }
  return `in=${inVal};out=${outVal};dur=${durVal};ttft=${ttftVal};win=${winVal};ratio=${ratioVal}`;
});

function formatCompactInteger(value?: number | null) {
  if (value == null || !Number.isFinite(value)) { return ""; }
  if (value >= 1_000_000) { return `${Math.round(value / 1_000_000)}M`; }
  if (value >= 1_000) { return `${Math.round(value / 1_000)}K`; }
  return String(value);
}

function formatTokenSpeed(value?: number | null) {
  if (value == null || !Number.isFinite(value)) { return "—"; }
  return `${Math.round(value)} t/s`;
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
          <!-- Wave-15 S1：「输入」口径 = 当前上下文占用（最近一次模型调用的 input_tokens），
               非会话累计（S2 保留在消息级 tokenCount / 统计汇总）。 -->
          <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-input-tokens">
            <Tooltip text="输入">
              <ArrowUp class="h-3 w-3 text-stone-400" />
            </Tooltip>
            <Tooltip text="上下文输入（最近一次模型调用）">
              <span class="inline-block min-w-[2.25rem] text-right">{{ formatCompactInteger(inputTokensTotal) || "0" }}</span>
            </Tooltip>
          </span>
          <Tooltip text="输出">
            <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-output-tokens">
              <ArrowDown class="h-3 w-3 text-stone-400" />
              <span class="inline-block min-w-[2.25rem] text-right">{{ formatCompactInteger(outputTokensTotal) || "0" }}</span>
            </span>
          </Tooltip>
          <Tooltip text="缓存读取">
            <span class="inline-flex items-center gap-1 tabular-nums" data-testid="status-metric-cache-tokens">
              <Zap class="h-3 w-3 text-stone-400" />
              <span class="inline-block min-w-[2.25rem] text-right">{{ formatCompactInteger(cacheHitTokensTotal) || "0" }}</span>
              <span v-if="cacheHitRatio" class="inline-block min-w-[2.25rem] text-right text-stone-400">· {{ cacheHitRatio }}</span>
            </span>
          </Tooltip>
          <!-- 速度位在无数据时渲染 "—"（无数字文本节点），故预留宽度落在承载 span 自身，
               使其在 9 / 12 / 1234 三档（含注入探针）下宽度恒定。 -->
          <Tooltip text="生成速度">
            <span
              class="inline-flex min-w-[4rem] items-center gap-1 tabular-nums"
              data-testid="status-metric-token-speed"
              :data-speed-mode="tokenSpeedCalculationMode || 'none'"
              :data-speed-debug="debugAttrsEnabled ? speedDebugString : undefined"
            >
              <Gauge class="h-3 w-3 text-stone-400" />
              {{ formatTokenSpeed(tokenGenerationSpeed) }}
            </span>
          </Tooltip>
        </div>
      </div>
    </section>
  </section>
</template>
