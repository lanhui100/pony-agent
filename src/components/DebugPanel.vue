<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { Bug, Check, ChevronRight, Copy } from "lucide-vue-next";

defineProps<{ active: boolean }>();
const emit = defineEmits<{ toggle: [] }>();

type DebugEntry = Record<string, unknown> & {
  event?: string;
  at?: number;
};

type StatusItem = {
  label: string;
  value: string;
  tone?: "warn" | "muted";
};

type EventCountItem = {
  event: string;
  count: number;
};

type RecentEventItem = {
  id: string;
  event: string;
  timestamp: string;
  count: number;
  summary: string;
  tone?: "warn" | "muted";
};

const debugEntries = ref<DebugEntry[]>([]);
const copiedKey = ref("");
const isDev = import.meta.env.DEV;
let copiedTimer: number | null = null;

const MAX_DEBUG_ENTRIES = 120;
const RECENT_SOURCE_LIMIT = 36;
const RECENT_EVENT_LIMIT = 12;
const EVENT_COUNT_LIMIT = 8;

const DEBUG_FIELD_LABELS: Record<string, string> = {
  anchorToComposerDistance: "anchor间距",
  anchorTop: "anchor顶",
  behavior: "行为",
  composerTop: "输入框顶",
  delayMs: "延迟",
  distanceFromBottom: "距底部",
  distanceToBottom: "距底部",
  expectedOverrideVersion: "覆盖版本",
  key: "按键",
  latestAgentTop: "最新Agent顶",
  latestUserBelowViewport: "User在视口下方",
  latestUserTop: "最新User顶",
  latestVisibleTurnLayoutSignature: "布局签名",
  programmaticScrollTargetMode: "目标模式",
  reason: "原因",
  requestId: "请求",
  showScrollToBottom: "底部按钮",
  signature: "消息签名",
  streamAutoFollowEnabled: "自动跟随",
  submitting: "提交中",
  targetDelta: "目标差值",
  targetTop: "目标位置"
};

const SUMMARY_FIELDS = [
  "reason",
  "behavior",
  "anchorToComposerDistance",
  "anchorTop",
  "composerTop",
  "latestUserBelowViewport",
  "latestUserTop",
  "latestAgentTop",
  "programmaticScrollTargetMode",
  "targetDelta",
  "distanceFromBottom",
  "distanceToBottom",
  "latestVisibleTurnLayoutSignature",
  "targetTop",
  "delayMs",
  "key",
  "requestId",
  "expectedOverrideVersion",
  "showScrollToBottom",
  "submitting",
  "signature"
] as const;

function formatTimestamp(ts: number) {
  const d = new Date(ts);
  return `${d.getHours().toString().padStart(2, "0")}:${d.getMinutes().toString().padStart(2, "0")}:${d.getSeconds().toString().padStart(2, "0")}.${d.getMilliseconds().toString().padStart(3, "0")}`;
}

function formatValue(value: unknown) {
  if (typeof value === "boolean") {
    return value ? "是" : "否";
  }
  if (value == null || value === "") {
    return "—";
  }
  if (typeof value === "number") {
    return Number.isInteger(value) ? String(value) : value.toFixed(1);
  }
  return String(value);
}

function formatMetric(value: unknown) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "—";
  }
  return `${Math.round(value)}px`;
}

function appendDebugEntry(entry: DebugEntry) {
  debugEntries.value = [...debugEntries.value.slice(-(MAX_DEBUG_ENTRIES - 1)), entry];
}

function getEventTone(entry: DebugEntry): "warn" | "muted" | undefined {
  if (entry.streamAutoFollowEnabled === false) {
    return "warn";
  }
  const event = String(entry.event ?? "");
  if (event.includes("skip") || event.includes("defer") || event.includes("cancelled")) {
    return "muted";
  }
  return undefined;
}

function getRecentEventSummary(entry: DebugEntry) {
  const parts = SUMMARY_FIELDS.flatMap((field) => {
    if (!(field in entry)) {
      return [];
    }
    return `${DEBUG_FIELD_LABELS[field] ?? field} ${formatValue(entry[field])}`;
  });
  return parts.slice(0, 3).join(" / ") || "无附加信息";
}

function getCopyFields(entry: DebugEntry) {
  return SUMMARY_FIELDS.flatMap((field) => {
    if (!(field in entry)) {
      return [];
    }
    return `  ${DEBUG_FIELD_LABELS[field] ?? field}: ${formatValue(entry[field])}`;
  });
}

function buildDebugCopyText() {
  if (!debugEntries.value.length) return "";
  const parts = debugEntries.value.map((entry) => {
    const timestamp = typeof entry.at === "number" ? formatTimestamp(entry.at) : "--:--:--.---";
    const lines = [`[${timestamp}] ${String(entry.event ?? "unknown")}`];
    lines.push(...getCopyFields(entry));
    return lines.join("\n");
  });
  return "=== Auto-Scroll Debug Log ===\n\n" + parts.join("\n\n");
}

function copyDebug() {
  const text = buildDebugCopyText();
  if (!text.trim()) return;
  void navigator.clipboard.writeText(text);
  copiedKey.value = "debug-all";
  if (copiedTimer != null) {
    window.clearTimeout(copiedTimer);
  }
  copiedTimer = window.setTimeout(() => {
    copiedKey.value = "";
    copiedTimer = null;
  }, 1400);
}

function handleScrollDebugEvent(event: Event) {
  const customEvent = event as CustomEvent<DebugEntry>;
  appendDebugEntry(customEvent.detail);
}

function loadExistingDebugEntries() {
  if (typeof window !== "undefined") {
    const w = window as typeof window & { __ponyScrollDebugBuffer?: DebugEntry[] };
    const buffer = w.__ponyScrollDebugBuffer;
    if (buffer) {
      debugEntries.value = [...buffer].slice(-MAX_DEBUG_ENTRIES);
    }
  }
}

const latestEntry = computed(() => {
  const entries = debugEntries.value;
  return entries.length ? entries[entries.length - 1] : null;
});

const statusItems = computed<StatusItem[]>(() => {
  const entry = latestEntry.value;
  if (!entry) return [];
  return [
    {
      label: "跟随",
      value: entry.streamAutoFollowEnabled === false ? "关闭" : "开启",
      tone: entry.streamAutoFollowEnabled === false ? "warn" : undefined
    },
    {
      label: "滚动中",
      value: entry.programmaticScrollActive ? "是" : "否"
    },
    {
      label: "滚动排队",
      value: entry.scrollQueued ? "是" : "否"
    },
    {
      label: "anchor间距",
      value: formatMetric(entry.anchorToComposerDistance),
      tone: typeof entry.anchorToComposerDistance === "number" && entry.anchorToComposerDistance < 0 ? "warn" : undefined
    },
    {
      label: "anchor顶",
      value: formatMetric(entry.anchorTop)
    },
    {
      label: "输入框顶",
      value: formatMetric(entry.composerTop)
    },
    {
      label: "按钮显示",
      value: formatValue(entry.showScrollToBottom),
      tone: entry.showScrollToBottom ? "warn" : undefined
    },
    {
      label: "User在视口下方",
      value: formatValue(entry.latestUserBelowViewport)
    },
    {
      label: "最新User顶",
      value: formatMetric(entry.latestUserTop)
    },
    {
      label: "最新Agent顶",
      value: formatMetric(entry.latestAgentTop)
    },
    {
      label: "目标模式",
      value: formatValue(entry.programmaticScrollTargetMode)
    },
    {
      label: "目标差值",
      value: formatMetric(entry.targetDelta),
      tone: typeof entry.targetDelta === "number" && Math.abs(entry.targetDelta) > 24 ? "warn" : undefined
    },
    {
      label: "scrollTop",
      value: formatMetric(entry.viewportScrollTop)
    },
    {
      label: "布局签名",
      value: formatValue(entry.latestVisibleTurnLayoutSignature)
    }
  ];
});

const diagnosisItems = computed<StatusItem[]>(() => {
  const visibilityEntry = [...debugEntries.value]
    .reverse()
    .find((entry) => String(entry.event ?? "") === "viewport-scroll:user-away-from-bottom");

  return [
    {
      label: "最近判定",
      value: visibilityEntry
        ? `${String(visibilityEntry.event ?? "unknown")} @ ${formatTimestamp((visibilityEntry.at as number) ?? Date.now())}`
        : "未发现"
    },
    {
      label: "按钮显示",
      value: formatValue(visibilityEntry?.showScrollToBottom)
    },
    {
      label: "User在视口下方",
      value: formatValue(visibilityEntry?.latestUserBelowViewport)
    },
    {
      label: "User位置",
      value: formatMetric(visibilityEntry?.latestUserTop)
    }
  ];
});

const topEventCounts = computed<EventCountItem[]>(() => {
  const counts = new Map<string, number>();
  for (const entry of debugEntries.value) {
    const event = String(entry.event ?? "unknown");
    counts.set(event, (counts.get(event) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, EVENT_COUNT_LIMIT)
    .map(([event, count]) => ({ event, count }));
});

const recentEvents = computed<RecentEventItem[]>(() => {
  const recent = debugEntries.value.slice(-RECENT_SOURCE_LIMIT);
  const grouped: RecentEventItem[] = [];

  for (const entry of recent) {
    const event = String(entry.event ?? "unknown");
    const timestamp = typeof entry.at === "number" ? formatTimestamp(entry.at) : "--:--:--.---";
    const summary = getRecentEventSummary(entry);
    const tone = getEventTone(entry);
    const previous = grouped[grouped.length - 1];

    if (previous && previous.event === event && previous.summary === summary) {
      previous.count += 1;
      previous.timestamp = timestamp;
      previous.id = `${event}-${entry.at ?? grouped.length}`;
      continue;
    }

    grouped.push({
      id: `${event}-${entry.at ?? grouped.length}`,
      event,
      timestamp,
      count: 1,
      summary,
      tone
    });
  }

  return grouped.slice(-RECENT_EVENT_LIMIT).reverse();
});

onMounted(() => {
  loadExistingDebugEntries();
  if (typeof window !== "undefined") {
    window.addEventListener("pony:workspace-scroll-debug", handleScrollDebugEvent);
  }
});

onBeforeUnmount(() => {
  if (typeof window !== "undefined") {
    window.removeEventListener("pony:workspace-scroll-debug", handleScrollDebugEvent);
  }
  if (copiedTimer != null) {
    window.clearTimeout(copiedTimer);
    copiedTimer = null;
  }
});
</script>

<template>
  <section v-if="isDev" class="collapsible-shell border-b border-stone-200/60 pb-4" :data-open="active">
    <button class="group flex w-full items-center justify-between gap-3 text-left" type="button" @click="emit('toggle')">
      <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
        <Bug class="h-3.5 w-3.5" />
        <span>DEBUG</span>
      </div>
      <div class="flex items-center gap-1">
        <button
          class="invisible group-hover:visible inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7f1e7] hover:text-stone-600"
          type="button"
          @click.stop="copyDebug()"
        >
          <component :is="copiedKey === 'debug-all' ? Check : Copy" class="h-3 w-3" />
        </button>
        <ChevronRight
          class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200"
          :class="{ 'rotate-90': active }"
        />
      </div>
    </button>

    <div class="collapsible-body">
      <div class="collapsible-content mt-2 space-y-3">
        <div v-if="debugEntries.length === 0" class="px-1 text-[10px] leading-5 text-stone-400">
          暂无 debug 事件
        </div>

        <template v-else>
          <section class="rounded-[0.65rem] border border-stone-200/70 bg-stone-50/60 px-2.5 py-2">
            <div class="mb-1 text-[10px] uppercase tracking-[0.14em] text-stone-400">当前状态</div>
            <div class="grid grid-cols-2 gap-x-3 gap-y-1">
              <div v-for="item in statusItems" :key="item.label" class="min-w-0 text-[10px] leading-[1.35]">
                <div class="text-stone-400">{{ item.label }}</div>
                <div class="truncate text-stone-700" :class="{ 'font-medium text-amber-700': item.tone === 'warn' }">
                  {{ item.value }}
                </div>
              </div>
            </div>
          </section>

          <section class="rounded-[0.65rem] border border-stone-200/70 px-2.5 py-2">
            <div class="mb-1 text-[10px] uppercase tracking-[0.14em] text-stone-400">按钮显示判定</div>
            <div class="space-y-1">
              <div v-for="item in diagnosisItems" :key="item.label" class="text-[10px] leading-[1.35]">
                <span class="text-stone-400">{{ item.label }}:</span>
                <span class="ml-1 text-stone-700">{{ item.value }}</span>
              </div>
            </div>
          </section>

          <section class="rounded-[0.65rem] border border-stone-200/70 px-2.5 py-2">
            <div class="mb-1 text-[10px] uppercase tracking-[0.14em] text-stone-400">高频事件</div>
            <div class="space-y-1">
              <div v-for="item in topEventCounts" :key="item.event" class="flex items-center gap-2 text-[10px] leading-[1.35]">
                <span class="w-5 shrink-0 text-right font-mono text-stone-400">{{ item.count }}</span>
                <span class="min-w-0 truncate text-stone-700">{{ item.event }}</span>
              </div>
            </div>
          </section>

          <section class="rounded-[0.65rem] border border-stone-200/70 px-2.5 py-2">
            <div class="mb-1 text-[10px] uppercase tracking-[0.14em] text-stone-400">最近事件</div>
            <div class="space-y-1.5">
              <div v-for="item in recentEvents" :key="item.id" class="border-b border-stone-100 pb-1.5 last:border-b-0 last:pb-0">
                <div class="flex items-center gap-1.5 text-[10px] leading-[1.2]">
                  <span class="shrink-0 font-mono text-stone-400">{{ item.timestamp }}</span>
                  <span class="truncate font-medium text-stone-700" :class="{ 'text-amber-700': item.tone === 'warn', 'text-stone-500': item.tone === 'muted' }">
                    {{ item.event }}
                  </span>
                  <span v-if="item.count > 1" class="shrink-0 rounded-full bg-stone-100 px-1 py-[1px] font-mono text-[9px] text-stone-500">
                    x{{ item.count }}
                  </span>
                </div>
                <div class="mt-0.5 pl-2 text-[10px] leading-[1.3] text-stone-500 [overflow-wrap:anywhere]">{{ item.summary }}</div>
              </div>
            </div>
          </section>
        </template>
      </div>
    </div>
  </section>
</template>

<style scoped>
.collapsible-shell > .collapsible-body {
  display: grid;
  grid-template-rows: 0fr;
  min-height: 0;
  opacity: 0;
  overflow: hidden;
  transition:
    grid-template-rows 260ms cubic-bezier(0.2, 0.72, 0.18, 1),
    opacity 180ms ease;
}

.collapsible-shell[data-open="true"] > .collapsible-body {
  grid-template-rows: 1fr;
  min-height: 0;
  opacity: 1;
}

.collapsible-content {
  min-height: 0;
  overflow: hidden;
}
</style>
