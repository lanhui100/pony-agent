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
import {
  formatBurstExcludedSuffix,
  formatSpeedSampleLabel
} from "@/lib/runtime/token-metrics";
import { isDebugLoggingEnabled } from "@/lib/runtime/utils";
import { useRuntimeStore } from "@/stores/runtime";

const props = defineProps<{
  sessionId: string;
  turnCount: number;
  modelCallCount: number;
  toolCallCount: number;
  copied: boolean;
  /** wave-54 C4：上下文占用（末次调用 contextInputTokens，回退 S1）；无数据/零值 ⇒ 该位点整个不渲染（wave-61 §2 E-2/E-5）。 */
  inputTokensTotal: number | null;
  outputTokensTotal: number | null;
  cacheHitTokensTotal: number | null;
  cacheHitRatio: string;
  tokenGenerationSpeed: number | null;
  tokenSpeedCalculationMode?: string;
  speedDebugInfo?: {
    /** 聚合范围（`turn_weighted` | `call_only` | `none`）。 */
    mode?: string;
    /** 速度口径（v2 I10）：`stream_decode` | `end_to_end` | `none`。 */
    basis?: string;
    /** 参与求和的已完成调用数。 */
    calls?: number;
    /** F1 I4：该轮 provider 调用**总数**（含无效/不可解码记录）；旧 payload 缺键 ⇒ null。 */
    callsTotal?: number | null;
    /** F1 I4：`decode_eligible` 记录数（进入解码口径分子/分母的样本）。 */
    callsEligible?: number | null;
    /**
     * F1 I4：`valid − decode_eligible`——因突发整包/无 TTFT/单 token 被剔除、
     * 只能进 `end_to_end` 回退的调用数。`> 0` 时显示端必须可辨别（本组件用 tooltip + `data-*`）。
     */
    burstExcluded?: number | null;
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

/**
 * 速度口径（v2 I10）：`stream_decode` = 解码窗口 `Σ(out−1)/Σgen`；`end_to_end` = 端到端
 * `Σout/Σdur`；`none` = 无值。口径与聚合范围（`tokenSpeedCalculationMode`）不同域，
 * 禁止用 mode 兜底推导 basis。
 */
const speedBasis = computed(() => props.speedDebugInfo?.basis ?? "none");

/** wave-54 C3：估算来源必须显式降级（`≈`），不得冒充真值。 */
const isEstimatedUsage = computed(() => props.speedDebugInfo?.origin === "estimated_chars");

/**
 * F1 I4：突发剔除数（`valid − decode_eligible`，直取快照，组件内不重算判据）。
 *
 * `> 0` ⇒ 本轮的 t/s **不是**纯解码速度：被物理下限（突发整包）/无 TTFT/单 token 剔除的调用
 * 只能进 `end_to_end` 回退。显示端必须可辨别；`null`/`0` ⇒ 不提示（旧 payload 无该键，
 * 不伪造「无突发」）。
 */
const speedBurstExcluded = computed(() => {
  const value = props.speedDebugInfo?.burstExcluded;
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.trunc(value) : 0;
});

/** F1 I4：样本量文本（`eligible/total 次调用`；旧 payload 缺 total ⇒ 只写参与数）。 */
const speedSampleText = computed(() =>
  formatSpeedSampleLabel(props.speedDebugInfo?.calls, props.speedDebugInfo?.callsTotal)
);

const speedTooltip = computed(() => {
  const callScope = props.speedDebugInfo?.mode === "call_only";
  // F1 I4：样本量（含剔除数）+ 口径必须写在 tooltip 文案里（行内位点只保留「图标 + 值」，
  // wave-61 §1 L-4/R1 冻结了行内子节点结构，故诚实化信息走 tooltip + `data-*`）。
  const burstSuffix = formatBurstExcludedSuffix(speedBurstExcluded.value);
  const sample = speedSampleText.value || "0 次调用";
  if (speedBasis.value === "stream_decode") {
    return callScope
      ? "当前调用的生成速度（解码口径：首 token 已计入 TTFT）"
      : `生成速度（${sample}，Σ(out−1)/Σ生成耗时${burstSuffix}）`;
  }
  if (speedBasis.value === "end_to_end") {
    return callScope
      ? "当前调用的整体速度（端到端口径，含排队与 prefill）"
      : `整体速度（${sample}，Σoutput/Σ端到端耗时，含排队与 prefill${burstSuffix}）`;
  }
  // basis 缺席但仍有速度值（旧 wire）：口径未知必须明说，不得默认成解码口径。
  if (isPositiveFiniteNumber(props.tokenGenerationSpeed)) {
    return `速度（口径未知，${sample}${burstSuffix}）`;
  }
  return "速度（暂无已完成调用）";
});

/**
 * C6 `data-speed-debug`：`basis=…;calls=…;in=…;out=…;dur=…;ttft=…;gen=…;win=…;ratio=…;origin=…`。
 *
 * wave-metrics（task-4）：`ratio` 不在本组件内重算 —— 投影层（快照 `contextRatio` 或 trace 反推）
 * 已是唯一来源（契约 §0「禁止各自重算」）；本组件只做字面量拼装，位点缺值一律 `-`。
 * 字面量格式/键序/分隔符逐字冻结（wave-61 §1 L-2 与 e2e 断言依赖）。
 */
const speedDebugString = computed(() => {
  const info = props.speedDebugInfo;
  const inVal = info?.inputTokens ?? props.inputTokensTotal ?? "-";
  const outVal = info?.outputTokens ?? props.outputTokensTotal ?? "-";
  const durVal = info?.durationMs ?? "-";
  const ttftVal = info?.ttftMs ?? "-";
  const genVal = info?.genMs ?? "-";
  const winVal = info?.contextWindow ?? props.contextWindowTokens ?? "-";
  const ratioVal = info?.ratio ?? "-";
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
 * wave-61 §2 E-1/E-2/E-3：位点可见性判定 —— 空值 / 零值 / 非有限值一律「不渲染」，
 * 禁止占位文本 `—` / `0` / `≈—`（§2 E-5）。计数位与 token 位都要求「有限且 > 0」。
 */
function isPositiveFiniteNumber(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

/** §2 E-3：圆环位仅在「已有用量（>0） + 合法窗口（>0）」同时成立时渲染。 */
const showContextRing = computed(
  () =>
    // wave-61 §2 E-3/E-5：零值同样视为「无值」—— 0 token 不得渲染成 0% 空环
    // （用户诉求「零值不显示」对圆环位同样成立；有值才入场）。
    isPositiveFiniteNumber(props.contextDisplayTokens) &&
    isPositiveFiniteNumber(props.contextWindowTokens)
);

/**
 * §1 L-2 圆环语义：百分比 `clamp(round(tokens / window * 100), 1, 100)`。
 * 比例 <1% 时收敛到下限 1%（不得渲染空环）；超窗时收敛到上限 100%。
 */
const contextUsagePercentage = computed(() => {
  const tokens = props.contextDisplayTokens;
  const windowTokens = props.contextWindowTokens;
  if (tokens == null || !Number.isFinite(tokens) || !isPositiveFiniteNumber(windowTokens)) {
    return 0;
  }
  const ratio = (tokens / windowTokens) * 100;
  return Math.min(100, Math.max(0, ratio < 1 && ratio > 0 ? 1 : Math.round(ratio)));
});

/**
 * §1 L-3 / 裁决 R9（Reviewer-B BLOCKING 处置）：位点缺席时**收列**，不留空列。
 * 保留 `grid-cols-4` class（冻结结构断言需要），仅在可见位点数 <4 时用内联 style 覆盖列模板，
 * 使「空值不渲染」不再产生 46% 行宽的视觉空洞（toolCallCount=0 是会话早期常态）。
 * 取舍：两行的严格列中心对齐只在两行可见位点数相同时成立（G-4c 的 4/4 夹具即此态）。
 */
function rowGridStyle(visibleSlotCount: number): Record<string, string> | undefined {
  if (visibleSlotCount <= 0 || visibleSlotCount >= 4) {
    return undefined;
  }
  return { gridTemplateColumns: `repeat(${visibleSlotCount}, minmax(0, 1fr))` };
}

/** §1 L-2 圆环 tooltip 的紧凑整数（与迁移前 composer 文案逐字保真）。 */
function formatCompactTokenInteger(value: number) {
  if (value >= 1_000_000) { return `${Math.round(value / 1_000_000)}M`; }
  if (value >= 1_000) { return `${Math.round(value / 1_000)}K`; }
  return String(value);
}

/** §1 L-2：速度信息在 tooltip 文案里的既有尾段（无速度值时不出现）。 */
const ringSpeedSuffix = computed(() => {
  const speed = props.tokenGenerationSpeed;
  if (!isPositiveFiniteNumber(speed)) { return ""; }
  const mode = props.tokenSpeedCalculationMode ?? "none";
  return ` · 速度: ${Math.round(speed)} t/s (${mode})`;
});

/** §1 L-2：圆环 tooltip 文案语义（`上下文用量：{pct}% · {已用} / {窗口}` + 既有速度尾段）。 */
const contextUsageTooltip = computed(() => {
  const tokens = props.contextDisplayTokens;
  const windowTokens = props.contextWindowTokens;
  if (tokens == null || !isPositiveFiniteNumber(windowTokens)) {
    return `上下文用量：暂无数据${ringSpeedSuffix.value}`;
  }
  const exactPercentage = (tokens / windowTokens) * 100;
  const displayPercent = exactPercentage > 0 && exactPercentage < 1 ? "<1" : String(Math.round(exactPercentage));
  return `上下文用量：${displayPercent}% · ${formatCompactTokenInteger(tokens)} / ${formatCompactTokenInteger(windowTokens)}${ringSpeedSuffix.value}`;
});

/**
 * §1 L-2 `data-context-debug`：`in=…;win=…;pct=…;mode=…;speed=…`（字面量格式逐字不变）。
 */
const contextDebugString = computed(() => {
  const inVal = props.contextDisplayTokens ?? "-";
  const winVal = props.contextWindowTokens ?? "-";
  const pctVal = contextUsagePercentage.value;
  const modeVal = props.tokenSpeedCalculationMode ?? "none";
  const speedVal = isPositiveFiniteNumber(props.tokenGenerationSpeed)
    ? Math.round(props.tokenGenerationSpeed)
    : "-";
  return `in=${inVal};win=${winVal};pct=${pctVal};mode=${modeVal};speed=${speedVal}`;
});

/**
 * wave-54 C6 紧凑格式：`<1000` 原值；`<1e6` 一位小数 `1.9K`；`≥1e6` 一位小数 `1.2M`。
 * wave-61 §2 E-5：`—` 分支已作废 —— 无值位点整个不渲染，故本函数只在有值时被调用。
 */
function formatCompactInteger(value: number) {
  if (value >= 1_000_000) { return `${(value / 1_000_000).toFixed(1)}M`; }
  if (value >= 1_000) { return `${(value / 1_000).toFixed(1)}K`; }
  return String(value);
}

/**
 * 速度显示（v2 §2.6）：`≈` 前缀 = 估算来源**或**端到端口径，两者同时成立只出现一次 `≈`；
 * 无值时位点不渲染（§2 E-2）。
 */
function formatTokenSpeed(value: number | null | undefined) {
  if (!isPositiveFiniteNumber(value)) { return ""; }
  const text = `${Math.round(value)} t/s`;
  return isEstimatedUsage.value || speedBasis.value === "end_to_end" ? `≈${text}` : text;
}

/**
 * wave-54 C3：估算标记覆盖 token 三位（输入/输出/缓存），不只速度位。
 * 无值时返回空串（位点不渲染，§2 E-2/E-5）。
 */
function formatTokenValue(value: number | null | undefined) {
  if (!isPositiveFiniteNumber(value)) { return ""; }
  const text = formatCompactInteger(value);
  return isEstimatedUsage.value ? `≈${text}` : text;
}

/**
 * wave-61 §2 E-2（裁决 R1）：缓存命中率**移出行内**，并入缓存位的值 tooltip
 * （行内只剩「图标 + 值」两个节点，避免尾段撑破列宽）。
 * 比率空串时不渲染比率段 —— 但位点本身仍按 token 值是否有效决定（不连带消失）。
 */
const cacheValueTooltip = computed(() => {
  const tokens = props.cacheHitTokensTotal;
  const ratio = props.cacheHitRatio?.trim() ?? "";
  if (!isPositiveFiniteNumber(tokens)) { return "缓存读取"; }
  const compact = `${isEstimatedUsage.value ? "≈" : ""}${formatCompactInteger(tokens)}`;
  return ratio
    ? `缓存读取：${compact}（会话累计读取）· 命中率 ${ratio}`
    : `缓存读取：${compact}（会话累计读取，命中率暂无数据）`;
});

/** §2 E-4：逐行可见位点数（用于整行折叠）。 */
const visibleCountSlotCount = computed(() => {
  const counts = [props.turnCount, props.modelCallCount, props.toolCallCount].filter(isPositiveFiniteNumber);
  return counts.length + (showContextRing.value ? 1 : 0);
});

const visibleTokenSlotCount = computed(
  () =>
    [
      props.inputTokensTotal,
      props.outputTokensTotal,
      props.cacheHitTokensTotal,
      props.tokenGenerationSpeed
    ].filter(isPositiveFiniteNumber).length
);

/**
 * §2 E-4（裁决 R5）：两行皆空 ⇒ 卡内容区与外层分隔段整段不渲染（面板头部保留）；
 * 但 `governance-block-banner` 不受连坐 —— 横幅在场时卡容器保留，仅折叠两行指标行。
 */
const showMetricsCard = computed(
  () => visibleCountSlotCount.value > 0 || visibleTokenSlotCount.value > 0 || blockDetail.value != null
);
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
      </div>
    </div>

    <section v-if="showMetricsCard" class="border-t border-stone-200/60 pt-3 mt-3">
      <div class="rounded-[0.45rem] bg-[#f6f0e8] px-3 py-2">
        <!--
          Wave-61 §1 L-1..L-5：计数行升级为 4 等列栅格，**上下文占用圆环作为第 4 个位点**
          （圆环唯一合法归属地，composer 内已彻底移除）；token 行同步改为 4 等列栅格，
          两行列中心物理对齐（`grid-cols-4` + `gap-x-2`，§1 L-3/L-5）。
          位点结构 = 「图标 + 值」两个子节点，`gap-1.5` 即图标↔值的亲密性（§1 L-4）；
          宽度预留（`min-w-*`）上移到**位点容器自身**，位点内部不得再有预留盒 / 右对齐。
          Wave-17 C2：位数变化只消耗容器预留宽度，不横向推移相邻位点（`tabular-nums` 在容器上）。
          Wave-61 §2 E-1/E-2/E-3：空值 / 零值 / 非有限值 ⇒ 整个位点不渲染（禁止 `—` / `0` 占位）。
          Wave-61 §3 M-1：位点携带静态 class `status-slot-enter`（圆环另加 `--ring`）承载入场动效。
        -->
        <div
          v-if="visibleCountSlotCount > 0"
          class="grid grid-cols-4 items-center gap-x-2 gap-y-0 text-[11px] leading-5 text-stone-600"
          :style="rowGridStyle(visibleCountSlotCount)"
          data-testid="status-metrics-counts"
        >
          <Tooltip v-if="isPositiveFiniteNumber(turnCount)" text="对话轮次数">
            <span
              class="status-slot-enter inline-flex min-w-[3rem] items-center justify-center gap-1.5 tabular-nums"
              data-testid="status-metric-turn-count"
            >
              <MessageSquareMore class="h-3 w-3 text-stone-400" />
              <span>{{ turnCount }}</span>
            </span>
          </Tooltip>
          <Tooltip v-if="isPositiveFiniteNumber(modelCallCount)" text="模型调用次数">
            <span
              class="status-slot-enter inline-flex min-w-[3rem] items-center justify-center gap-1.5 tabular-nums"
              data-testid="status-metric-model-call-count"
            >
              <Orbit class="h-3 w-3 text-stone-400" />
              <span>{{ modelCallCount }}</span>
            </span>
          </Tooltip>
          <Tooltip v-if="isPositiveFiniteNumber(toolCallCount)" text="工具调用次数">
            <span
              class="status-slot-enter inline-flex min-w-[3rem] items-center justify-center gap-1.5 tabular-nums"
              data-testid="status-metric-tool-call-count"
            >
              <Wrench class="h-3 w-3 text-stone-400" />
              <span>{{ toolCallCount }}</span>
            </span>
          </Tooltip>
          <!-- §1 L-1/L-2：上下文占用圆环 —— 计数行第 4 位点，testid 不带 `status-metric-` 前缀；
               §2 E-3：无用量 / 窗口非法时整个位点不渲染。 -->
          <Tooltip v-if="showContextRing" :text="contextUsageTooltip">
            <span
              class="status-slot-enter status-slot-enter--ring inline-flex min-w-[3rem] cursor-help items-center justify-center gap-1.5 tabular-nums"
              data-testid="status-context-usage"
              tabindex="0"
              aria-label="上下文用量"
              :data-context-debug="debugAttrsEnabled ? contextDebugString : undefined"
            >
              <svg class="h-3.5 w-3.5 -rotate-90 transform" viewBox="0 0 20 20" aria-hidden="true">
                <circle
                  cx="10"
                  cy="10"
                  r="7.5"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.4"
                  class="opacity-20"
                />
                <circle
                  cx="10"
                  cy="10"
                  r="7.5"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.4"
                  stroke-linecap="round"
                  :stroke-dasharray="2 * Math.PI * 7.5"
                  :stroke-dashoffset="(2 * Math.PI * 7.5) * (1 - contextUsagePercentage / 100)"
                  class="status-ring-arc transition-all duration-500 ease-out text-stone-500 hover:text-stone-700"
                />
              </svg>
              <!-- wave-61.1（用户追加诉求）：圆环后内联展示百分比，使圆环位不再只能靠 hover 读懂；
                   `contextUsagePercentage` 已 clamp(1..100)，故不会出现 `0%` 空环文案（§2 E-5）。 -->
              <span class="status-ring-percent text-stone-600" data-testid="status-context-usage-percent">{{ contextUsagePercentage }}%</span>
            </span>
          </Tooltip>
        </div>

        <!-- Token metrics：与计数行同一 4 等列栅格（§1 L-3）；`border-t` 仍挂在 token 行自身，
             使本行保持为 `bg-[#f6f0e8]` 指标卡的直接子节点（wave-59 F2-4 不回归）。
             §2 E-4：可见 token 位数为 0 时本行连同 `border-t` 一并折叠。 -->
        <div
          v-if="visibleTokenSlotCount > 0"
          class="mt-2 grid grid-cols-4 items-center gap-x-2 gap-y-0 border-t border-stone-200/60 pt-2 text-[11px] leading-5 text-stone-600"
          :style="rowGridStyle(visibleTokenSlotCount)"
          data-testid="status-metrics-tokens"
        >
          <!-- wave-54 C4/C6：「输入」口径 = 末次调用 contextInputTokens（回退 S1），
               估算来源加 `≈`。非会话累计；无值不渲染（§2 E-2/E-5）。 -->
          <span
            v-if="isPositiveFiniteNumber(inputTokensTotal)"
            class="status-slot-enter inline-flex min-w-[3rem] items-center justify-center gap-1.5 tabular-nums"
            data-testid="status-metric-input-tokens"
          >
            <Tooltip text="输入">
              <ArrowUp class="h-3 w-3 text-stone-400" />
            </Tooltip>
            <Tooltip text="上下文输入（最近一次模型调用，协议归一口径）">
              <span>{{ formatTokenValue(inputTokensTotal) }}</span>
            </Tooltip>
          </span>
          <Tooltip v-if="isPositiveFiniteNumber(outputTokensTotal)" text="输出">
            <span
              class="status-slot-enter inline-flex min-w-[3rem] items-center justify-center gap-1.5 tabular-nums"
              data-testid="status-metric-output-tokens"
            >
              <ArrowDown class="h-3 w-3 text-stone-400" />
              <span>{{ formatTokenValue(outputTokensTotal) }}</span>
            </span>
          </Tooltip>
          <!-- §2 E-2（裁决 R1）：命中率尾段已移出行内，并入下方「值」tooltip；
               行内只保留「图标 + 值」两个节点。 -->
          <span
            v-if="isPositiveFiniteNumber(cacheHitTokensTotal)"
            class="status-slot-enter inline-flex min-w-[3rem] items-center justify-center gap-1.5 tabular-nums"
            data-testid="status-metric-cache-tokens"
          >
            <Tooltip text="缓存读取">
              <Zap class="h-3 w-3 text-stone-400" />
            </Tooltip>
            <Tooltip :text="cacheValueTooltip">
              <!-- R1 副作用补偿：命中率已移入本 tooltip，必须保留键盘可达（tabindex + cursor-help）。 -->
              <span tabindex="0" class="cursor-help">{{ formatTokenValue(cacheHitTokensTotal) }}</span>
            </Tooltip>
          </span>
          <!-- §1 L-4：速度位预留下限 3.5rem（Lead 实测：4.25rem 会撑破 4 列 track），
               预留落在位点容器自身，值节点不带 min-w / text-right。 -->
          <span
            v-if="isPositiveFiniteNumber(tokenGenerationSpeed)"
            class="status-slot-enter inline-flex min-w-[3.5rem] items-center justify-center gap-1.5 whitespace-nowrap tabular-nums"
            data-testid="status-metric-token-speed"
            :data-speed-mode="tokenSpeedCalculationMode || 'none'"
            :data-speed-basis="speedBasis"
            :data-speed-calls="speedDebugInfo?.calls ?? undefined"
            :data-speed-calls-total="speedDebugInfo?.callsTotal ?? undefined"
            :data-speed-burst-excluded="speedBurstExcluded > 0 ? speedBurstExcluded : undefined"
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

<style scoped>
/* Wave-61 §3 M-1/M-3：位点入场动效 —— 只动合成层（opacity / transform），
   不驱动 width/height/margin/gap；位点由 `v-if` 挂载即重播（无 leave，§3 M-2）。 */
@keyframes status-slot-enter-in {
  from {
    opacity: 0;
    transform: translateY(4px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* §3 M-1：圆环位额外 `scale(0.92) → 1`。 */
@keyframes status-slot-enter-ring {
  from {
    opacity: 0;
    transform: translateY(4px) scale(0.92);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

.status-slot-enter {
  animation: status-slot-enter-in 200ms cubic-bezier(0.3, 0.7, 0.4, 1) both;
}

.status-slot-enter--ring {
  animation: status-slot-enter-ring 200ms cubic-bezier(0.3, 0.7, 0.4, 1) both;
}

/* §3 M-4 双轨降级 · 轨道一：系统媒体查询（与 src/styles.css §3.5 同一约定）。 */
@media (prefers-reduced-motion: reduce) {
  .status-slot-enter,
  .status-slot-enter--ring {
    animation: none !important;
    transition: none !important;
    opacity: 1 !important;
    transform: none !important;
  }

  /* §3 M-4：降级不仅覆盖入场，弧的进度过渡同样关闭（否则每次进度更新仍扫 500ms）。 */
  .status-ring-arc {
    transition: none !important;
  }
}

/* §3 M-4 双轨降级 · 轨道二：应用内根属性 `data-pds-reduce-motion="true"`（挂在 documentElement）。 */
[data-pds-reduce-motion="true"] .status-slot-enter,
[data-pds-reduce-motion="true"] .status-slot-enter--ring {
  animation: none !important;
  transition: none !important;
  opacity: 1 !important;
  transform: none !important;
}

[data-pds-reduce-motion="true"] .status-ring-arc {
  transition: none !important;
}
</style>
