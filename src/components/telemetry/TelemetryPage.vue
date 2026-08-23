<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { ArrowLeft } from "lucide-vue-next";
import ModelMonitorPage from "@/components/ModelMonitorPage.vue";
import TraceInspector from "@/components/TraceInspector.vue";
import { useSettingsStore } from "@/stores/settings";

/**
 * PA-096：二级遥测页。
 *
 * - coding 模式：[Trace, 指标] 双 tab，默认 Trace；
 *   work 模式：仅 [指标] 单 tab（原始 turn 调试明细不可达——用户决策 2026-08-22）。
 * - tab 可用性收敛：启动竞态（默认 coding → 进入 Trace → loadSettings resolve 为
 *   work）时当前 tab 自动落到首个可用 tab，不弹回 home。
 * - APG tabs：roving tabindex + 方向键；进入页面聚焦标题（out-in 切换后焦点落点）。
 */

type TelemetryTab = "trace" | "metrics";

const emit = defineEmits<{
  (event: "navigate", page: "home"): void;
}>();

const settingsStore = useSettingsStore();
const { workspaceMode } = storeToRefs(settingsStore);

const isCoding = computed(() => workspaceMode.value === "coding");

interface TelemetryTabItem {
  id: TelemetryTab;
  label: string;
}

const availableTabs = computed<TelemetryTabItem[]>(() =>
  isCoding.value
    ? [
        { id: "trace", label: "Trace" },
        { id: "metrics", label: "指标" }
      ]
    : [{ id: "metrics", label: "指标" }]
);

const activeTab = ref<TelemetryTab>(isCoding.value ? "trace" : "metrics");

watch(availableTabs, (tabs) => {
  if (!tabs.some((tab) => tab.id === activeTab.value)) {
    activeTab.value = tabs[0]!.id;
  }
});

const headingRef = ref<HTMLElement | null>(null);
const tabRefs = ref<HTMLButtonElement[]>([]);

function setTabRef(element: unknown, index: number) {
  if (element instanceof HTMLButtonElement) {
    tabRefs.value[index] = element;
  }
}

function selectTab(tabId: TelemetryTab) {
  activeTab.value = tabId;
}

function focusTab(index: number) {
  const tabs = availableTabs.value;
  if (!tabs.length) {
    return;
  }
  const nextIndex = ((index % tabs.length) + tabs.length) % tabs.length;
  activeTab.value = tabs[nextIndex]!.id;
  tabRefs.value[nextIndex]?.focus();
}

function handleTablistKeydown(event: KeyboardEvent) {
  const currentIndex = availableTabs.value.findIndex((tab) => tab.id === activeTab.value);
  switch (event.key) {
    case "ArrowRight":
    case "ArrowDown":
      event.preventDefault();
      focusTab(currentIndex + 1);
      break;
    case "ArrowLeft":
    case "ArrowUp":
      event.preventDefault();
      focusTab(currentIndex - 1);
      break;
    case "Home":
      event.preventDefault();
      focusTab(0);
      break;
    case "End":
      event.preventDefault();
      focusTab(availableTabs.value.length - 1);
      break;
    default:
      break;
  }
}

onMounted(() => {
  headingRef.value?.focus();
});
</script>

<template>
  <section
    class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.6rem] border border-stone-200/70 bg-white/72"
    data-testid="telemetry-page"
  >
    <div class="flex shrink-0 items-center justify-between gap-3 border-b border-stone-200/70 px-4 py-3">
      <div class="flex min-w-0 items-center gap-2">
        <button
          type="button"
          class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[0.35rem] text-stone-500 transition hover:bg-[#f7e3bf] hover:text-stone-900"
          aria-label="返回对话"
          title="返回对话"
          data-testid="telemetry-back"
          @click="emit('navigate', 'home')"
        >
          <ArrowLeft class="h-4 w-4" />
        </button>
        <h2
          ref="headingRef"
          class="truncate text-sm font-semibold tracking-[-0.02em] text-stone-950 outline-none"
          tabindex="-1"
          data-testid="telemetry-heading"
        >
          {{ isCoding ? "遥测" : "指标" }}
        </h2>
        <span v-if="isCoding" class="hidden text-[11px] leading-5 text-stone-500 sm:inline">
          Trace 与指标的二级读面，不随对话页常驻
        </span>
        <span v-else class="hidden text-[11px] leading-5 text-stone-500 sm:inline">
          模型调用与延迟聚合读面
        </span>
      </div>

      <div
        class="flex items-center gap-1 rounded-[0.5rem] bg-[#f6f0e8] p-1"
        role="tablist"
        aria-label="遥测视图切换"
        data-testid="telemetry-tablist"
        @keydown="handleTablistKeydown"
      >
        <button
          v-for="(tab, index) in availableTabs"
          :id="`telemetry-tab-${tab.id}`"
          :key="tab.id"
          :ref="(element) => setTabRef(element, index)"
          type="button"
          role="tab"
          :aria-selected="activeTab === tab.id"
          :aria-controls="`telemetry-panel-${tab.id}`"
          :tabindex="activeTab === tab.id ? 0 : -1"
          class="rounded-[0.35rem] px-3 py-1.5 text-[12px] font-medium transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
          :class="
            activeTab === tab.id
              ? 'bg-white text-stone-900 shadow-[0_1px_2px_rgba(28,25,23,0.08)]'
              : 'text-stone-500 hover:text-stone-900'
          "
          :data-testid="`telemetry-tab-${tab.id}`"
          @click="selectTab(tab.id)"
        >
          {{ tab.label }}
        </button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1 flex-col p-3">
      <div
        v-if="activeTab === 'trace'"
        id="telemetry-panel-trace"
        role="tabpanel"
        aria-labelledby="telemetry-tab-trace"
        class="flex min-h-0 flex-1 flex-col"
        data-testid="telemetry-panel-trace"
      >
        <TraceInspector class="min-h-0 flex-1" />
      </div>

      <div
        v-else
        id="telemetry-panel-metrics"
        role="tabpanel"
        aria-labelledby="telemetry-tab-metrics"
        class="flex min-h-0 flex-1 flex-col"
        data-testid="telemetry-panel-metrics"
      >
        <ModelMonitorPage :embedded="true" class="min-h-0 flex-1" />
      </div>
    </div>
  </section>
</template>
