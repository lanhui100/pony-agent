<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type {
  CapabilitySourceView,
  CapabilityView,
  ModelMonitorSessionDrilldownView,
  ModelMonitorSessionRow,
  ModelMonitorSummaryView,
  TraceTimelineEntry,
  TurnTraceRecord
} from "@/types/runtime";

const loadingSummary = ref(false);
const loadingDrilldown = ref(false);
const summaryError = ref<string | null>(null);
const drilldownError = ref<string | null>(null);
const summary = ref<ModelMonitorSummaryView | null>(null);
const selectedSessionId = ref<string | null>(null);
const selectedTurnId = ref<string | null>(null);
const drilldown = ref<ModelMonitorSessionDrilldownView | null>(null);
const latestDrilldownRequestId = ref(0);
const loadingCapabilitySources = ref(false);
const loadingCapabilities = ref(false);
const capabilityError = ref<string | null>(null);
const capabilitySources = ref<CapabilitySourceView[]>([]);
const capabilities = ref<CapabilityView[]>([]);
const selectedCapabilitySourceId = ref<string | null>(null);
const selectedCapabilityId = ref<string | null>(null);
const selectedCapability = ref<CapabilityView | null>(null);

const overviewCards = computed(() => {
  const overview = summary.value?.overview;
  if (!overview) {
    return [];
  }

  return [
    {
      key: "requests",
      label: "请求总数",
      value: formatInteger(overview.requestCount),
      detail: `${formatInteger(overview.sessionCount)} 个会话`
    },
    {
      key: "tokens",
      label: "总 Tokens",
      value: formatInteger(overview.totalTokens),
      detail: `输入 ${formatInteger(overview.inputTokens)} / 输出 ${formatInteger(overview.outputTokens)}`
    },
    {
      key: "latency",
      label: "平均首 Token 延迟",
      value: formatDurationMs(overview.avgFirstTokenLatencyMs),
      detail: `平均总耗时 ${formatDurationMs(overview.avgTurnDurationMs)}`
    },
    {
      key: "retrieval",
      label: "检索参与",
      value: formatInteger(overview.retrievalParticipationCount),
      detail: `失败 ${formatInteger(overview.failedRequestCount)} / 工具 ${formatInteger(overview.toolCallCount)}`
    }
  ];
});

const selectedTrace = computed<TurnTraceRecord | null>(() => {
  const traces = drilldown.value?.runtimeView.session.turnTraceHistory ?? [];
  if (!traces.length) {
    return null;
  }

  if (!selectedTurnId.value) {
    return traces[0] ?? null;
  }

  return traces.find((trace) => trace.turnId === selectedTurnId.value) ?? traces[0] ?? null;
});

const selectedTimeline = computed<TraceTimelineEntry[]>(() => {
  return selectedTrace.value?.traceTimeline ?? [];
});

const selectedCapabilityActivities = computed(() => {
  return (selectedTrace.value?.toolActivities ?? []).filter((activity) => activity.capabilityInvocation);
});

const selectedSessionMetrics = computed<ModelMonitorSessionRow | null>(() => {
  return drilldown.value?.metrics ?? null;
});

const selectedCapabilitySource = computed<CapabilitySourceView | null>(() => {
  const sourceId = selectedCapabilitySourceId.value;
  if (!sourceId) {
    return null;
  }

  return capabilitySources.value.find((source) => source.sourceId === sourceId) ?? null;
});

async function loadSummary() {
  if (!isTauriAvailable()) {
    summaryError.value = "当前环境未连接 Tauri 后端，监控读面不可用。";
    summary.value = null;
    return;
  }

  loadingSummary.value = true;
  summaryError.value = null;
  try {
    const payload = await safeInvoke<ModelMonitorSummaryView>("load_model_monitor_summary");
    summary.value = payload;
    const firstSessionId = payload.sessions[0]?.sessionId ?? null;
    if (!selectedSessionId.value && firstSessionId) {
      await loadDrilldown(firstSessionId);
    } else if (selectedSessionId.value && !payload.sessions.some((row) => row.sessionId === selectedSessionId.value)) {
      selectedSessionId.value = null;
      drilldown.value = null;
      selectedTurnId.value = null;
    }
  } catch (error) {
    summaryError.value = toErrorMessage(error, "加载监控摘要失败");
    summary.value = null;
  } finally {
    loadingSummary.value = false;
  }
}

async function loadCapabilitySources() {
  if (!isTauriAvailable()) {
    capabilityError.value = "当前环境未连接 Tauri 后端，capability 调试读面不可用。";
    capabilitySources.value = [];
    capabilities.value = [];
    selectedCapabilitySourceId.value = null;
    selectedCapabilityId.value = null;
    selectedCapability.value = null;
    return;
  }

  loadingCapabilitySources.value = true;
  capabilityError.value = null;
  try {
    const payload = await safeInvoke<CapabilitySourceView[]>("list_capability_sources");
    capabilitySources.value = payload;
    const firstSourceId = payload[0]?.sourceId ?? null;
    if (!selectedCapabilitySourceId.value || !payload.some((source) => source.sourceId === selectedCapabilitySourceId.value)) {
      selectedCapabilitySourceId.value = firstSourceId;
    }

    if (selectedCapabilitySourceId.value) {
      await loadCapabilities(selectedCapabilitySourceId.value);
    } else {
      capabilities.value = [];
      selectedCapabilityId.value = null;
      selectedCapability.value = null;
    }
  } catch (error) {
    capabilityError.value = toErrorMessage(error, "加载 capability sources 失败");
    capabilitySources.value = [];
    capabilities.value = [];
    selectedCapabilitySourceId.value = null;
    selectedCapabilityId.value = null;
    selectedCapability.value = null;
  } finally {
    loadingCapabilitySources.value = false;
  }
}

async function loadCapabilities(sourceId: string) {
  if (!isTauriAvailable()) {
    return;
  }

  loadingCapabilities.value = true;
  capabilityError.value = null;
  selectedCapabilitySourceId.value = sourceId;
  try {
    const payload = await safeInvoke<CapabilityView[]>("list_capabilities", {
      sourceId,
      kind: null
    });
    capabilities.value = payload;
    const firstCapabilityId = payload[0]?.capabilityId ?? null;
    if (!selectedCapabilityId.value || !payload.some((capability) => capability.capabilityId === selectedCapabilityId.value)) {
      selectedCapabilityId.value = firstCapabilityId;
    }

    if (selectedCapabilityId.value) {
      await loadCapabilityDetail(selectedCapabilityId.value);
    } else {
      selectedCapability.value = null;
    }
  } catch (error) {
    capabilityError.value = toErrorMessage(error, "加载 capabilities 失败");
    capabilities.value = [];
    selectedCapabilityId.value = null;
    selectedCapability.value = null;
  } finally {
    loadingCapabilities.value = false;
  }
}

async function loadCapabilityDetail(capabilityId: string) {
  if (!isTauriAvailable()) {
    return;
  }

  selectedCapabilityId.value = capabilityId;
  try {
    selectedCapability.value = await safeInvoke<CapabilityView | null>("inspect_capability", {
      capabilityId
    });
  } catch (error) {
    capabilityError.value = toErrorMessage(error, "加载 capability 详情失败");
    selectedCapability.value = null;
  }
}

async function loadDrilldown(sessionId: string) {
  if (!isTauriAvailable()) {
    drilldownError.value = "当前环境未连接 Tauri 后端，无法查看会话下钻。";
    return;
  }

  loadingDrilldown.value = true;
  drilldownError.value = null;
  selectedSessionId.value = sessionId;
  const requestId = latestDrilldownRequestId.value + 1;
  latestDrilldownRequestId.value = requestId;
  try {
    const payload = await safeInvoke<ModelMonitorSessionDrilldownView>(
      "load_model_monitor_session_drilldown",
      { sessionId }
    );
    if (requestId !== latestDrilldownRequestId.value) {
      return;
    }
    drilldown.value = payload;
    selectedTurnId.value = payload.runtimeView.session.turnTraceHistory?.[0]?.turnId ?? null;
  } catch (error) {
    if (requestId !== latestDrilldownRequestId.value) {
      return;
    }
    drilldownError.value = toErrorMessage(error, "加载会话下钻失败");
    drilldown.value = null;
    selectedTurnId.value = null;
  } finally {
    if (requestId === latestDrilldownRequestId.value) {
      loadingDrilldown.value = false;
    }
  }
}

function selectTurn(turnId: string) {
  selectedTurnId.value = turnId;
}

function formatInteger(value?: number | null) {
  return new Intl.NumberFormat("zh-CN").format(value ?? 0);
}

function formatDurationMs(value?: number | null) {
  if (value == null) {
    return "--";
  }

  if (value < 1000) {
    return `${Math.round(value)} ms`;
  }

  return `${(value / 1000).toFixed(2)} s`;
}

function formatTimestamp(value?: number | null) {
  if (value == null) {
    return "--";
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(value);
}

function summarizeTrace(trace: TurnTraceRecord) {
  const parts = [
    `${formatInteger(trace.totalTokens ?? 0)} tokens`,
    formatDurationMs(trace.turnDurationMs)
  ];
  if (trace.providerModel) {
    parts.unshift(trace.providerModel);
  }
  return parts.join(" · ");
}

function timelineSummary(entry: TraceTimelineEntry) {
  const parts: string[] = [entry.kind];
  if (entry.providerModel) {
    parts.push(entry.providerModel);
  }
  if (entry.totalTokens != null) {
    parts.push(`${formatInteger(entry.totalTokens)} tokens`);
  }
  if (entry.turnDurationMs != null) {
    parts.push(formatDurationMs(entry.turnDurationMs));
  }
  return parts.join(" · ");
}

function toErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

onMounted(() => {
  void loadSummary();
  void loadCapabilitySources();
});
</script>

<template>
  <section
    class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.85rem] border border-stone-200/70 bg-white/82 px-5 py-5 sm:px-6"
  >
    <div class="flex flex-wrap items-start justify-between gap-4 border-b border-stone-200/70 pb-4">
      <div>
        <p class="text-[11px] uppercase tracking-[0.22em] text-stone-400">Model Monitor</p>
        <h2 class="mt-2 text-[1.55rem] font-semibold tracking-[-0.03em] text-stone-950">模型监控</h2>
        <p class="mt-2 max-w-3xl text-[13px] leading-6 text-stone-500">
          聚合当前本地会话的请求量、延迟、缓存、检索参与度，并提供按会话下钻的 trace 证据。
        </p>
      </div>

      <button
        type="button"
        class="rounded-full border border-stone-300/80 px-4 py-2 text-[12px] font-medium text-stone-700 transition hover:border-stone-400 hover:bg-stone-50 disabled:cursor-not-allowed disabled:opacity-60"
        data-testid="model-monitor-refresh"
        :disabled="loadingSummary || loadingDrilldown"
        @click="loadSummary"
      >
        {{ loadingSummary ? "刷新中..." : "刷新摘要" }}
      </button>
    </div>

    <div v-if="summaryError" class="mt-5 rounded-[0.8rem] border border-rose-200 bg-rose-50 px-4 py-3 text-sm text-rose-700" data-testid="model-monitor-summary-error">
      {{ summaryError }}
    </div>

    <div v-else-if="loadingSummary && !summary" class="mt-5 rounded-[0.8rem] border border-stone-200 bg-stone-50 px-4 py-8 text-center text-sm text-stone-500" data-testid="model-monitor-summary-loading">
      正在加载监控摘要...
    </div>

    <div v-else-if="summary" class="flex min-h-0 flex-1 flex-col">
      <div class="mt-5 grid gap-4 lg:grid-cols-4" data-testid="model-monitor-overview">
        <section
          v-for="card in overviewCards"
          :key="card.key"
          class="rounded-[0.85rem] border border-stone-200/70 bg-[#faf6ef] px-4 py-4"
        >
          <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">{{ card.label }}</div>
          <div class="mt-3 text-[1.8rem] font-semibold tracking-[-0.04em] text-stone-950">{{ card.value }}</div>
          <p class="mt-2 text-[12px] leading-5 text-stone-500">{{ card.detail }}</p>
        </section>
      </div>

      <div class="mt-5 grid gap-4 xl:grid-cols-[minmax(240px,0.8fr)_minmax(0,1.2fr)]">
        <section class="rounded-[0.85rem] border border-stone-200/70 bg-white/75">
          <div class="flex items-center justify-between gap-3 border-b border-stone-200/70 px-4 py-4">
            <div>
              <div class="text-sm font-semibold text-stone-900">Capability Sources</div>
              <p class="mt-1 text-[12px] text-stone-500">统一 capability ingress 读面</p>
            </div>
            <button
              type="button"
              class="rounded-full border border-stone-300/80 px-3 py-1.5 text-[11px] font-medium text-stone-700 transition hover:border-stone-400 hover:bg-stone-50 disabled:cursor-not-allowed disabled:opacity-60"
              data-testid="model-monitor-capability-refresh"
              :disabled="loadingCapabilitySources || loadingCapabilities"
              @click="loadCapabilitySources"
            >
              {{ loadingCapabilitySources ? "刷新中..." : "刷新 capabilities" }}
            </button>
          </div>

          <div v-if="capabilityError" class="px-4 py-4 text-sm text-rose-700" data-testid="model-monitor-capability-error">
            {{ capabilityError }}
          </div>

          <div
            v-else-if="loadingCapabilitySources && capabilitySources.length === 0"
            class="px-4 py-6 text-sm text-stone-500"
            data-testid="model-monitor-capability-loading"
          >
            正在加载 capability sources...
          </div>

          <div v-else-if="capabilitySources.length === 0" class="px-4 py-6 text-sm text-stone-500" data-testid="model-monitor-capability-empty">
            当前没有可用的 capability source。
          </div>

          <div v-else class="divide-y divide-stone-200/70" data-testid="model-monitor-capability-sources">
            <button
              v-for="source in capabilitySources"
              :key="source.sourceId"
              type="button"
              class="flex w-full flex-col gap-1 px-4 py-3 text-left transition hover:bg-stone-50"
              :class="source.sourceId === selectedCapabilitySourceId ? 'bg-stone-50' : ''"
              :data-testid="`model-monitor-capability-source-${source.sourceId}`"
              @click="loadCapabilities(source.sourceId)"
            >
              <div class="flex items-center justify-between gap-3">
                <span class="text-sm font-medium text-stone-900">{{ source.displayName }}</span>
                <span class="rounded-full bg-stone-100 px-2 py-1 text-[10px] uppercase tracking-[0.16em] text-stone-500">{{ source.availability }}</span>
              </div>
              <div class="text-[12px] text-stone-500">{{ source.sourceId }}</div>
              <div class="text-[11px] text-stone-400">{{ source.declaredCapabilities.join(" / ") }}</div>
            </button>
          </div>
        </section>

        <section class="rounded-[0.85rem] border border-stone-200/70 bg-white/75">
          <div class="border-b border-stone-200/70 px-4 py-4">
            <div class="text-sm font-semibold text-stone-900">Capabilities</div>
            <p class="mt-1 text-[12px] text-stone-500">
              {{ selectedCapabilitySource ? `${selectedCapabilitySource.displayName} · ${selectedCapabilitySource.transportKind}` : "选择 source 查看 capability facts" }}
            </p>
          </div>

          <div v-if="loadingCapabilities && capabilities.length === 0" class="px-4 py-6 text-sm text-stone-500">
            正在加载 capability 列表...
          </div>

          <div v-else-if="capabilities.length === 0" class="px-4 py-6 text-sm text-stone-500">
            当前 source 下没有 capability 条目。
          </div>

          <div v-else class="grid gap-0 divide-y divide-stone-200/70" data-testid="model-monitor-capabilities">
            <button
              v-for="capability in capabilities"
              :key="capability.capabilityId"
              type="button"
              class="flex w-full flex-col gap-1 px-4 py-3 text-left transition hover:bg-stone-50"
              :class="capability.capabilityId === selectedCapabilityId ? 'bg-stone-50' : ''"
              :data-testid="`model-monitor-capability-${capability.capabilityId}`"
              @click="loadCapabilityDetail(capability.capabilityId)"
            >
              <div class="flex items-center justify-between gap-3">
                <span class="text-sm font-medium text-stone-900">{{ capability.label }}</span>
                <span class="rounded-full bg-stone-100 px-2 py-1 text-[10px] uppercase tracking-[0.16em] text-stone-500">{{ capability.kind }}</span>
              </div>
              <div class="text-[12px] text-stone-500">{{ capability.invocationMode }}</div>
              <div class="text-[11px] text-stone-400">
                {{ capability.requiresApproval ? "需要审批" : "无需审批" }} · {{ capability.hostMediated ? "host mediated" : "direct" }}
              </div>
            </button>
          </div>

          <div v-if="selectedCapability" class="border-t border-stone-200/70 bg-[#faf6ef] px-4 py-4" data-testid="model-monitor-capability-detail">
            <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Capability Inspect</div>
            <div class="mt-2 text-sm font-semibold text-stone-900">{{ selectedCapability.capabilityId }}</div>
            <p class="mt-2 text-[12px] leading-5 text-stone-600">{{ selectedCapability.description }}</p>
            <div class="mt-3 grid gap-2 text-[12px] text-stone-500 sm:grid-cols-2">
              <div>source: {{ selectedCapability.sourceId }}</div>
              <div>mode: {{ selectedCapability.invocationMode }}</div>
              <div>safety: {{ selectedCapability.safetyClass }}</div>
              <div>visibility: {{ selectedCapability.visibility }}</div>
              <div>permission: {{ selectedCapability.permissionScope }}</div>
            </div>
            <div class="mt-3 text-[11px] text-stone-400">
              tags: {{ selectedCapability.observabilityTags.join(", ") || "--" }}
            </div>
          </div>
        </section>
      </div>

      <div class="mt-5 grid min-h-0 flex-1 gap-4 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
        <section class="flex min-h-0 flex-col rounded-[0.85rem] border border-stone-200/70 bg-white/75">
          <div class="border-b border-stone-200/70 px-4 py-4">
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div>
                <div class="text-sm font-semibold text-stone-900">聚合视图</div>
                <p class="mt-1 text-[12px] text-stone-500">Provider / Model / Tool / Session 维度读面</p>
              </div>
              <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">
                生成于 {{ formatTimestamp(summary.generatedAtMs) }}
              </div>
            </div>
          </div>

          <div class="grid min-h-0 flex-1 gap-4 overflow-y-auto px-4 py-4 lg:grid-cols-2">
            <section class="rounded-[0.75rem] border border-stone-200/70 bg-stone-50/70 p-3" data-testid="model-monitor-providers">
              <div class="text-xs font-semibold uppercase tracking-[0.16em] text-stone-500">Providers</div>
              <div class="mt-3 space-y-2">
                <div v-for="row in summary.providers" :key="row.key" class="rounded-[0.7rem] bg-white px-3 py-3">
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-stone-900">{{ row.label }}</div>
                      <div class="mt-1 text-[12px] text-stone-500">
                        请求 {{ formatInteger(row.requestCount) }} · 检索 {{ formatInteger(row.retrievalParticipationCount) }}
                      </div>
                    </div>
                    <div class="text-right text-[12px] text-stone-500">
                      <div>{{ formatInteger(row.totalTokens) }} tokens</div>
                      <div class="mt-1">{{ formatDurationMs(row.avgTurnDurationMs) }}</div>
                    </div>
                  </div>
                </div>
                <div v-if="summary.providers.length === 0" class="text-[12px] text-stone-400">暂无 provider 聚合数据。</div>
              </div>

              <div class="mt-3 rounded-[0.7rem] bg-black/10 px-3 py-3" data-testid="model-monitor-capability-activity">
                <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Capability Activity</div>
                <div v-if="selectedCapabilityActivities.length === 0" class="mt-2 text-[12px] text-stone-400">
                  当前 trace 没有记录 capability 活动。
                </div>
                <div v-else class="mt-3 space-y-2">
                  <div
                    v-for="activity in selectedCapabilityActivities"
                    :key="activity.id"
                    class="rounded-[0.65rem] border border-white/10 bg-white/5 px-3 py-3"
                  >
                    <div class="flex items-start justify-between gap-3">
                      <div>
                        <div class="text-sm font-medium text-white">
                          {{ activity.capabilityInvocation?.capabilityId || activity.name }}
                        </div>
                        <div class="mt-1 text-[12px] text-stone-300">
                          {{ activity.capabilityInvocation?.sourceId || "unknown-source" }} ·
                          {{ activity.capabilityInvocation?.invocationMode || "unknown-mode" }}
                        </div>
                      </div>
                      <div class="text-right text-[11px] text-stone-400">
                        <div>{{ activity.status }}</div>
                        <div class="mt-1">{{ activity.capabilityInvocation?.failureKind || "ok" }}</div>
                      </div>
                    </div>
                    <div class="mt-2 text-[12px] leading-6 text-stone-300">{{ activity.summary }}</div>
                    <div class="mt-2 text-[11px] text-stone-400">
                      permission: {{ activity.capabilityInvocation?.permissionScope || "--" }} · approval:
                      {{ activity.capabilityInvocation?.requiresApproval ? "required" : "no" }}
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <section class="rounded-[0.75rem] border border-stone-200/70 bg-stone-50/70 p-3" data-testid="model-monitor-models">
              <div class="text-xs font-semibold uppercase tracking-[0.16em] text-stone-500">Models</div>
              <div class="mt-3 space-y-2">
                <div v-for="row in summary.models" :key="row.key" class="rounded-[0.7rem] bg-white px-3 py-3">
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-stone-900">{{ row.label }}</div>
                      <div class="mt-1 text-[12px] text-stone-500">
                        请求 {{ formatInteger(row.requestCount) }} · 模型调用 {{ formatInteger(row.modelCallCount) }}
                      </div>
                    </div>
                    <div class="text-right text-[12px] text-stone-500">
                      <div>{{ formatInteger(row.totalTokens) }} tokens</div>
                      <div class="mt-1">{{ formatDurationMs(row.avgFirstTokenLatencyMs) }}</div>
                    </div>
                  </div>
                </div>
                <div v-if="summary.models.length === 0" class="text-[12px] text-stone-400">暂无 model 聚合数据。</div>
              </div>
            </section>

            <section class="rounded-[0.75rem] border border-stone-200/70 bg-stone-50/70 p-3" data-testid="model-monitor-tools">
              <div class="text-xs font-semibold uppercase tracking-[0.16em] text-stone-500">Tools</div>
              <div class="mt-3 space-y-2">
                <div v-for="row in summary.tools" :key="row.key" class="rounded-[0.7rem] bg-white px-3 py-3">
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-stone-900">{{ row.label }}</div>
                      <div class="mt-1 text-[12px] text-stone-500">
                        调用 {{ formatInteger(row.callCount) }} · 失败 {{ formatInteger(row.failedCallCount) }}
                      </div>
                    </div>
                    <div class="text-right text-[12px] text-stone-500">
                      <div>均值 {{ formatDurationMs(row.avgDurationMs) }}</div>
                      <div class="mt-1">总计 {{ formatDurationMs(row.totalDurationMs) }}</div>
                    </div>
                  </div>
                </div>
                <div v-if="summary.tools.length === 0" class="text-[12px] text-stone-400">暂无 tool 聚合数据。</div>
              </div>
            </section>

            <section class="rounded-[0.75rem] border border-stone-200/70 bg-stone-50/70 p-3" data-testid="model-monitor-capability-sources-summary">
              <div class="text-xs font-semibold uppercase tracking-[0.16em] text-stone-500">Capability Sources</div>
              <div class="mt-3 space-y-2">
                <div v-for="row in summary.capabilitySources" :key="row.key" class="rounded-[0.7rem] bg-white px-3 py-3">
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-stone-900">{{ row.label }}</div>
                      <div class="mt-1 text-[12px] text-stone-500">
                        调用 {{ formatInteger(row.callCount) }} · 失败 {{ formatInteger(row.failedCallCount) }}
                      </div>
                    </div>
                    <div class="text-right text-[12px] text-stone-500">
                      <div>均值 {{ formatDurationMs(row.avgDurationMs) }}</div>
                      <div class="mt-1">总计 {{ formatDurationMs(row.totalDurationMs) }}</div>
                    </div>
                  </div>
                </div>
                <div v-if="summary.capabilitySources.length === 0" class="text-[12px] text-stone-400">暂无 capability source 聚合数据。</div>
              </div>
            </section>

            <section class="rounded-[0.75rem] border border-stone-200/70 bg-stone-50/70 p-3" data-testid="model-monitor-capability-invocation-modes-summary">
              <div class="text-xs font-semibold uppercase tracking-[0.16em] text-stone-500">Capability Invocation Modes</div>
              <div class="mt-3 space-y-2">
                <div v-for="row in summary.capabilityInvocationModes" :key="row.key" class="rounded-[0.7rem] bg-white px-3 py-3">
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-stone-900">{{ row.label }}</div>
                      <div class="mt-1 text-[12px] text-stone-500">
                        调用 {{ formatInteger(row.callCount) }} · 失败 {{ formatInteger(row.failedCallCount) }}
                      </div>
                    </div>
                    <div class="text-right text-[12px] text-stone-500">
                      <div>均值 {{ formatDurationMs(row.avgDurationMs) }}</div>
                      <div class="mt-1">总计 {{ formatDurationMs(row.totalDurationMs) }}</div>
                    </div>
                  </div>
                </div>
                <div v-if="summary.capabilityInvocationModes.length === 0" class="text-[12px] text-stone-400">暂无 capability invocation 聚合数据。</div>
              </div>
            </section>

            <section class="rounded-[0.75rem] border border-stone-200/70 bg-stone-50/70 p-3" data-testid="model-monitor-capability-failure-classes-summary">
              <div class="text-xs font-semibold uppercase tracking-[0.16em] text-stone-500">Capability Failure Classes</div>
              <div class="mt-3 space-y-2">
                <div v-for="row in summary.capabilityFailureClasses" :key="row.key" class="rounded-[0.7rem] bg-white px-3 py-3">
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-stone-900">{{ row.label }}</div>
                      <div class="mt-1 text-[12px] text-stone-500">
                        调用 {{ formatInteger(row.callCount) }} · 失败 {{ formatInteger(row.failedCallCount) }}
                      </div>
                    </div>
                    <div class="text-right text-[12px] text-stone-500">
                      <div>均值 {{ formatDurationMs(row.avgDurationMs) }}</div>
                      <div class="mt-1">总计 {{ formatDurationMs(row.totalDurationMs) }}</div>
                    </div>
                  </div>
                </div>
                <div v-if="summary.capabilityFailureClasses.length === 0" class="text-[12px] text-stone-400">暂无 capability failure 聚合数据。</div>
              </div>
            </section>

            <section class="rounded-[0.75rem] border border-stone-200/70 bg-stone-50/70 p-3" data-testid="model-monitor-sessions">
              <div class="text-xs font-semibold uppercase tracking-[0.16em] text-stone-500">Sessions</div>
              <div class="mt-3 space-y-2">
                <button
                  v-for="row in summary.sessions"
                  :key="row.sessionId"
                  type="button"
                  class="block w-full rounded-[0.7rem] border px-3 py-3 text-left transition"
                  :class="selectedSessionId === row.sessionId ? 'border-stone-900 bg-stone-900 text-white' : 'border-transparent bg-white hover:border-stone-300'"
                  :data-testid="`model-monitor-session-${row.sessionId}`"
                  @click="loadDrilldown(row.sessionId)"
                >
                  <div class="flex items-start justify-between gap-3">
                    <div class="min-w-0">
                      <div class="truncate text-sm font-medium">{{ row.title || row.sessionId }}</div>
                      <div class="mt-1 line-clamp-2 text-[12px]" :class="selectedSessionId === row.sessionId ? 'text-stone-300' : 'text-stone-500'">
                        {{ row.summary || "暂无摘要" }}
                      </div>
                    </div>
                    <div class="shrink-0 text-right text-[12px]" :class="selectedSessionId === row.sessionId ? 'text-stone-300' : 'text-stone-500'">
                      <div>{{ formatInteger(row.totalTokens) }} tokens</div>
                      <div class="mt-1">{{ formatTimestamp(row.updatedAtMs) }}</div>
                    </div>
                  </div>
                </button>
                <div v-if="summary.sessions.length === 0" class="text-[12px] text-stone-400">当前没有可展示的会话 telemetry。</div>
              </div>
            </section>
          </div>
        </section>

        <section class="flex min-h-0 flex-col rounded-[0.85rem] border border-stone-200/70 bg-stone-950 text-stone-100">
          <div class="border-b border-stone-800 px-4 py-4">
            <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Session Drill-down</div>
            <div class="mt-2 text-lg font-medium text-white" data-testid="model-monitor-drilldown-title">
              {{ selectedSessionMetrics?.title || selectedSessionId || "选择一个会话" }}
            </div>
            <p class="mt-2 text-[12px] leading-5 text-stone-300">
              {{ selectedSessionMetrics?.summary || "下钻会展示会话级指标、turn trace 以及 build-context 证据。" }}
            </p>
          </div>

          <div v-if="drilldownError" class="mx-4 mt-4 rounded-[0.75rem] border border-rose-500/40 bg-rose-500/10 px-3 py-3 text-sm text-rose-200" data-testid="model-monitor-drilldown-error">
            {{ drilldownError }}
          </div>

          <div v-else-if="loadingDrilldown && !drilldown" class="flex flex-1 items-center justify-center px-4 text-sm text-stone-400" data-testid="model-monitor-drilldown-loading">
            正在加载会话下钻...
          </div>

          <div v-else-if="drilldown && selectedSessionMetrics" class="grid min-h-0 flex-1 gap-4 overflow-y-auto px-4 py-4">
            <section class="grid gap-3 sm:grid-cols-2" data-testid="model-monitor-drilldown-metrics">
              <div class="rounded-[0.75rem] bg-white/5 px-3 py-3">
                <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">成本与负载</div>
                <div class="mt-2 text-sm text-white">
                  {{ formatInteger(selectedSessionMetrics.totalTokens) }} tokens
                </div>
                <div class="mt-1 text-[12px] text-stone-300">
                  请求 {{ formatInteger(selectedSessionMetrics.requestCount) }} · 模型 {{ formatInteger(selectedSessionMetrics.modelCallCount) }} · 工具 {{ formatInteger(selectedSessionMetrics.toolCallCount) }}
                </div>
              </div>
              <div class="rounded-[0.75rem] bg-white/5 px-3 py-3">
                <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">稳定性与检索</div>
                <div class="mt-2 text-sm text-white">
                  首 Token {{ formatDurationMs(selectedSessionMetrics.avgFirstTokenLatencyMs) }}
                </div>
                <div class="mt-1 text-[12px] text-stone-300">
                  检索参与 {{ formatInteger(selectedSessionMetrics.retrievalParticipationCount) }} · 失败 {{ formatInteger(selectedSessionMetrics.failedRequestCount) }}
                </div>
              </div>
            </section>

            <section class="rounded-[0.75rem] bg-white/5 px-3 py-3" data-testid="model-monitor-turn-list">
              <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Turns</div>
              <div class="mt-3 space-y-2">
                <button
                  v-for="trace in drilldown.runtimeView.session.turnTraceHistory ?? []"
                  :key="trace.turnId"
                  type="button"
                  class="block w-full rounded-[0.7rem] border px-3 py-3 text-left transition"
                  :class="selectedTurnId === trace.turnId ? 'border-white/30 bg-white/10' : 'border-transparent bg-black/10 hover:border-white/20'"
                  :data-testid="`model-monitor-turn-${trace.turnId}`"
                  @click="selectTurn(trace.turnId)"
                >
                  <div class="text-sm font-medium text-white">{{ trace.title || trace.turnId }}</div>
                  <div class="mt-1 text-[12px] text-stone-300">{{ summarizeTrace(trace) }}</div>
                </button>
                <div v-if="(drilldown.runtimeView.session.turnTraceHistory ?? []).length === 0" class="text-[12px] text-stone-400">
                  当前会话没有 trace 记录。
                </div>
              </div>
            </section>

            <section v-if="selectedTrace" class="rounded-[0.75rem] bg-white/5 px-3 py-3" data-testid="model-monitor-selected-trace">
              <div class="flex items-start justify-between gap-3">
                <div>
                  <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Selected Trace</div>
                  <div class="mt-2 text-sm font-medium text-white">{{ selectedTrace.title || selectedTrace.turnId }}</div>
                </div>
                <div class="text-right text-[12px] text-stone-300">
                  <div>{{ selectedTrace.providerName || selectedTrace.providerRequestedName || "unknown-provider" }}</div>
                  <div class="mt-1">{{ selectedTrace.providerModel || "unknown-model" }}</div>
                </div>
              </div>

              <div class="mt-3 grid gap-3 md:grid-cols-2">
                <div class="rounded-[0.7rem] bg-black/10 px-3 py-3">
                  <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Build Context</div>
                  <div class="mt-2 text-[12px] leading-6 text-stone-300">
                    消息数 {{ selectedTrace.buildContextObservation?.messageCount ?? 0 }} · 工具数
                    {{ selectedTrace.buildContextObservation?.toolCount ?? 0 }} · 图片数
                    {{ selectedTrace.buildContextObservation?.imageCount ?? 0 }}
                  </div>
                  <div class="mt-2 text-[12px] leading-6 text-stone-300">
                    {{ selectedTrace.buildContextObservation?.semiStableContextText || "暂无 build-context 摘要。" }}
                  </div>
                </div>

                <div class="rounded-[0.7rem] bg-black/10 px-3 py-3">
                  <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Return Summary</div>
                  <div class="mt-2 text-[12px] leading-6 text-stone-300">
                    {{ selectedTrace.sessionSummary || selectedTrace.fallbackReason || "暂无会话摘要。" }}
                  </div>
                  <div v-if="selectedTrace.error" class="mt-2 text-[12px] leading-6 text-rose-200">
                    错误: {{ selectedTrace.error }}
                  </div>
                </div>
              </div>

              <div class="mt-3 space-y-2" data-testid="model-monitor-trace-timeline">
                <div
                  v-for="entry in selectedTimeline"
                  :key="entry.id"
                  class="rounded-[0.7rem] border border-white/10 bg-black/10 px-3 py-3"
                >
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-white">{{ entry.label }}</div>
                      <div class="mt-1 text-[12px] text-stone-300">{{ timelineSummary(entry) }}</div>
                    </div>
                    <div class="text-[11px] uppercase tracking-[0.12em] text-stone-400">{{ entry.state }}</div>
                  </div>
                  <div v-if="entry.text || entry.reasoningContent || entry.error" class="mt-2 text-[12px] leading-6 text-stone-300">
                    {{ entry.text || entry.reasoningContent || entry.error }}
                  </div>
                </div>
                <div v-if="selectedTimeline.length === 0" class="text-[12px] text-stone-400">当前 trace 没有 timeline 明细。</div>
              </div>
            </section>
          </div>

          <div v-else class="flex flex-1 items-center justify-center px-4 text-center text-sm text-stone-400" data-testid="model-monitor-empty-drilldown">
            选择一个 session 行，查看该会话的 trace timeline 与 build-context 证据。
          </div>
        </section>
      </div>
    </div>
  </section>
</template>
