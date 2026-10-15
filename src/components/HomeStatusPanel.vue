<script setup lang="ts">
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

defineProps<{
  sessionId: string;
  turnCount: number;
  modelCallCount: number;
  toolCallCount: number;
  copied: boolean;
  inputTokensTotal: number;
  outputTokensTotal: number;
  cacheHitTokensTotal: number;
  cacheHitRatio: string;
  tokenGenerationSpeed: number | null;
  showContextUsage: boolean;
  contextDisplayTokens: number | null;
  contextWindowTokens: number | null;
  statusSummary: string;
  fallbackReason: string | null;
  error: string | null;
}>();

const emit = defineEmits<{ copySessionId: [] }>();

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
        <Tooltip text="对话轮次数">
          <span class="inline-flex items-center gap-1">
            <MessageSquareMore class="h-3 w-3 text-stone-400" />
            {{ turnCount }}
          </span>
        </Tooltip>
        <Tooltip text="模型调用次数">
          <span class="inline-flex items-center gap-1">
            <Orbit class="h-3 w-3 text-stone-400" />
            {{ modelCallCount }}
          </span>
        </Tooltip>
        <Tooltip text="工具调用次数">
          <span class="inline-flex items-center gap-1">
            <Wrench class="h-3 w-3 text-stone-400" />
            {{ toolCallCount }}
          </span>
        </Tooltip>
      </div>
    </div>

    <section class="border-t border-stone-200/60 pt-3 mt-3 space-y-1">
      <div class="rounded-[0.45rem] bg-[#f6f0e8] px-3 py-2 space-y-1.5">
        <!-- Token metrics -->
        <div class="flex items-center justify-around text-[11px] leading-5 text-stone-600">
          <Tooltip text="输入">
            <span class="inline-flex items-center gap-1">
              <ArrowUp class="h-3 w-3 text-stone-400" />
              {{ formatCompactInteger(inputTokensTotal) || "0" }}
            </span>
          </Tooltip>
          <Tooltip text="输出">
            <span class="inline-flex items-center gap-1">
              <ArrowDown class="h-3 w-3 text-stone-400" />
              {{ formatCompactInteger(outputTokensTotal) || "0" }}
            </span>
          </Tooltip>
          <Tooltip text="缓存读取">
            <span class="inline-flex items-center gap-1">
              <Zap class="h-3 w-3 text-stone-400" />
              {{ formatCompactInteger(cacheHitTokensTotal) || "0" }}
              <span v-if="cacheHitRatio" class="text-stone-400">· {{ cacheHitRatio }}</span>
            </span>
          </Tooltip>
          <Tooltip text="生成速度">
            <span class="inline-flex items-center gap-1">
              <Gauge class="h-3 w-3 text-stone-400" />
              {{ formatTokenSpeed(tokenGenerationSpeed) }}
            </span>
          </Tooltip>
        </div>
      </div>

      <!-- Session status messages -->
      <div
        v-if="statusSummary || fallbackReason || error"
        class="border-t border-stone-200/60 pt-1"
      >
        <div class="space-y-0.5">
          <div
            v-if="statusSummary"
            class="flex items-start gap-1.5 text-[11px] leading-4 text-stone-600"
            data-testid="status-session-summary"
          >
            {{ statusSummary }}
          </div>
          <p
            v-if="fallbackReason"
            class="flex items-start gap-1.5 text-[11px] leading-4 text-amber-800"
          >
            <AlertTriangle class="mt-0.5 h-3 w-3 shrink-0 text-amber-600" />
            <span>{{ fallbackReason }}</span>
          </p>
          <p
            v-if="error"
            class="flex items-start gap-1.5 text-[11px] leading-4 text-rose-700"
          >
            <AlertTriangle class="mt-0.5 h-3 w-3 shrink-0 text-rose-500" />
            <span>{{ error }}</span>
          </p>
        </div>
      </div>
    </section>
  </section>
</template>
