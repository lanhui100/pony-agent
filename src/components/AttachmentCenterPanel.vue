<script setup lang="ts">
import { computed, ref } from "vue";
import { RotateCcw, Search, Trash2 } from "lucide-vue-next";
import type { AttachmentAsset, AttachmentAssetStatus } from "@/types/runtime";

type CleanupRequestPayload = {
  sessionId: string | null;
  statuses: AttachmentAssetStatus[];
};

type CleanupTone = "default" | "warning" | "danger" | "success";

const STATUS_OPTIONS: Array<{ value: AttachmentAssetStatus; label: string }> = [
  { value: "active", label: "活跃" },
  { value: "missing_payload", label: "缺载荷" },
  { value: "expired", label: "已过期" },
  { value: "reclaimable", label: "可回收" }
];

const props = withDefaults(
  defineProps<{
    assets: AttachmentAsset[];
    currentSessionId?: string | null;
    cleanupConnected?: boolean;
    cleanupBusy?: boolean;
    cleanupMessage?: string;
    cleanupTone?: CleanupTone;
  }>(),
  {
    currentSessionId: null,
    cleanupConnected: false,
    cleanupBusy: false,
    cleanupMessage: "",
    cleanupTone: "default"
  }
);

const emit = defineEmits<{
  (event: "request-cleanup", payload: CleanupRequestPayload): void;
}>();

const searchQuery = ref("");
const statusFilters = ref<AttachmentAssetStatus[]>([]);
const cleanupStatuses = ref<AttachmentAssetStatus[]>(["reclaimable"]);

const normalizedAssets = computed(() =>
  [...props.assets].sort((left, right) => {
    if (right.createdAtMs !== left.createdAtMs) {
      return right.createdAtMs - left.createdAtMs;
    }

    return left.id.localeCompare(right.id);
  })
);

const currentSessionKey = computed(() => props.currentSessionId?.trim() || null);

const sessionAssets = computed(() => {
  if (!currentSessionKey.value) {
    return normalizedAssets.value;
  }

  return normalizedAssets.value.filter((asset) => asset.sessionId === currentSessionKey.value);
});

const normalizedSearchQuery = computed(() => searchQuery.value.trim().toLowerCase());

const searchFilteredAssets = computed(() => {
  if (!normalizedSearchQuery.value) {
    return sessionAssets.value;
  }

  return sessionAssets.value.filter((asset) => {
    const assetName = asset.name?.toLowerCase() ?? "";
    const relativePath = asset.relativePath.toLowerCase();
    const mimeType = asset.mimeType.toLowerCase();

    return (
      assetName.includes(normalizedSearchQuery.value) ||
      relativePath.includes(normalizedSearchQuery.value) ||
      mimeType.includes(normalizedSearchQuery.value)
    );
  });
});

const filteredAssets = computed(() => {
  if (statusFilters.value.length === 0) {
    return searchFilteredAssets.value;
  }

  const requestedStatuses = new Set(statusFilters.value);
  return searchFilteredAssets.value.filter((asset) => requestedStatuses.has(asset.status ?? "active"));
});

const visibleStatusCounts = computed(() => {
  const counts = new Map<AttachmentAssetStatus, number>();

  for (const option of STATUS_OPTIONS) {
    counts.set(option.value, 0);
  }

  for (const asset of searchFilteredAssets.value) {
    const status = asset.status ?? "active";
    counts.set(status, (counts.get(status) ?? 0) + 1);
  }

  return counts;
});

const quickStats = computed(() =>
  STATUS_OPTIONS.map((option) => ({
    ...option,
    count: sessionAssets.value.filter((asset) => (asset.status ?? "active") === option.value).length
  })).filter((item) => item.count > 0)
);

const headerSummary = computed(() => {
  const total = sessionAssets.value.length;
  const visible = filteredAssets.value.length;

  if (total === 0) {
    return currentSessionKey.value ? "当前会话还没有附件。" : "当前还没有附件。";
  }

  if (visible !== total) {
    return `当前会话 ${total} 个附件，已筛到 ${visible} 个。`;
  }

  return `当前会话 ${total} 个附件。`;
});

const hasSessionAssets = computed(() => sessionAssets.value.length > 0);
const recentAssets = computed(() => filteredAssets.value.slice(0, 6));

const cleanupSessionId = computed(() => currentSessionKey.value);

const cleanupCandidates = computed(() => {
  const requestedStatuses = new Set(cleanupStatuses.value);

  if (requestedStatuses.size === 0) {
    return [];
  }

  return sessionAssets.value.filter((asset) => requestedStatuses.has(asset.status ?? "active"));
});

function toggleStatusFilter(status: AttachmentAssetStatus) {
  if (statusFilters.value.includes(status)) {
    statusFilters.value = statusFilters.value.filter((value) => value !== status);
    return;
  }

  statusFilters.value = [...statusFilters.value, status];
}

function toggleCleanupStatus(status: AttachmentAssetStatus) {
  if (cleanupStatuses.value.includes(status)) {
    cleanupStatuses.value = cleanupStatuses.value.filter((value) => value !== status);
    return;
  }

  cleanupStatuses.value = [...cleanupStatuses.value, status];
}

function resetFilters() {
  searchQuery.value = "";
  statusFilters.value = [];
}

function requestCleanup() {
  emit("request-cleanup", {
    sessionId: cleanupSessionId.value,
    statuses: [...cleanupStatuses.value]
  });
}

function statusLabel(status?: AttachmentAssetStatus) {
  const value = status ?? "active";
  return STATUS_OPTIONS.find((option) => option.value === value)?.label ?? "活跃";
}

function statusBadgeClass(status?: AttachmentAssetStatus) {
  const value = status ?? "active";

  if (value === "reclaimable") {
    return "border-amber-200 text-amber-700";
  }

  if (value === "expired") {
    return "border-rose-200 text-rose-700";
  }

  if (value === "missing_payload") {
    return "border-stone-200 text-stone-600";
  }

  return "border-emerald-200 text-emerald-700";
}

function cleanupMessageClass(tone: CleanupTone) {
  if (tone === "danger") {
    return "text-rose-700";
  }

  if (tone === "warning") {
    return "text-amber-700";
  }

  if (tone === "success") {
    return "text-emerald-700";
  }

  return "text-stone-500";
}

function formatFileSize(sizeBytes: number) {
  if (sizeBytes < 1024) {
    return `${sizeBytes} B`;
  }

  if (sizeBytes < 1024 * 1024) {
    return `${(sizeBytes / 1024).toFixed(1)} KB`;
  }

  return `${(sizeBytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatDateTime(timestampMs?: number | null) {
  if (timestampMs == null) {
    return "--";
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(timestampMs);
}
</script>

<template>
  <div data-testid="attachment-center">
    <div class="space-y-3">
      <div class="space-y-1">
        <p class="text-[12px] leading-5 text-stone-600">
          {{ headerSummary }}
        </p>
        <div
          v-if="quickStats.length"
          class="flex flex-wrap gap-x-3 gap-y-1 pt-0.5 text-[10px] text-stone-500"
        >
          <span v-for="item in quickStats" :key="item.value">
            {{ item.label }} {{ item.count }}
          </span>
        </div>
      </div>

      <div v-if="recentAssets.length" class="divide-y divide-stone-100">
        <article
          v-for="asset in recentAssets"
          :key="asset.id"
          class="py-2 first:pt-0 last:pb-0"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="truncate text-[12px] font-medium text-stone-800">
                {{ asset.name || asset.relativePath }}
              </div>
              <div class="mt-0.5 flex flex-wrap gap-x-2 gap-y-1 text-[10px] text-stone-500">
                <span>{{ asset.mimeType }}</span>
                <span>{{ formatFileSize(asset.sizeBytes) }}</span>
                <span>引用 {{ asset.referenceCount ?? 0 }}</span>
                <span>{{ formatDateTime(asset.createdAtMs) }}</span>
              </div>
            </div>
            <span
              :class="statusBadgeClass(asset.status)"
              class="shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-medium"
            >
              {{ statusLabel(asset.status) }}
            </span>
          </div>
        </article>
      </div>
      <p v-else-if="hasSessionAssets" class="text-[12px] leading-5 text-stone-500">
        当前会话没有符合条件的附件。
      </p>

      <div v-if="hasSessionAssets" class="space-y-3 border-t border-stone-200/60 pt-3">
        <div class="flex items-center justify-between gap-2">
          <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.16em] text-stone-500">
            <Search class="h-3.5 w-3.5" />
            <span>筛选</span>
          </div>
          <button
            class="inline-flex items-center gap-1 text-[10px] text-stone-500 transition hover:text-stone-700"
            type="button"
            @click="resetFilters"
          >
            <RotateCcw class="h-3 w-3" />
            <span>重置</span>
          </button>
        </div>

        <label class="block space-y-1 text-[11px] text-stone-500">
          <span>搜索名称、路径或 MIME</span>
          <input
            v-model="searchQuery"
            data-testid="attachment-name-filter"
            class="w-full border-b border-stone-200 bg-transparent px-0 py-2 text-[12px] text-stone-700 outline-none transition placeholder:text-stone-400 focus:border-stone-300"
            placeholder="输入关键词"
            type="text"
          />
        </label>

        <div class="flex flex-wrap gap-1.5">
          <button
            v-for="option in STATUS_OPTIONS"
            :key="option.value"
            :aria-pressed="statusFilters.includes(option.value)"
            :data-testid="`attachment-status-${option.value}`"
            :class="
              statusFilters.includes(option.value)
                ? 'border-stone-900 text-stone-900'
                : 'border-stone-200 text-stone-500 hover:border-stone-300 hover:text-stone-700'
            "
            class="rounded-full border px-2 py-1 text-[10px] transition"
            type="button"
            @click="toggleStatusFilter(option.value)"
          >
            {{ option.label }} {{ visibleStatusCounts.get(option.value) ?? 0 }}
          </button>
        </div>

        <div class="space-y-2 border-t border-stone-200/60 pt-3">
          <div class="flex items-start justify-between gap-3">
            <div class="space-y-1">
              <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.16em] text-stone-500">
                <Trash2 class="h-3.5 w-3.5" />
                <span>Cleanup</span>
              </div>
              <p class="text-[11px] leading-5 text-stone-500">
                当前会话有 {{ cleanupCandidates.length }} 个可清理候选。
              </p>
            </div>
            <button
              data-testid="attachment-cleanup-trigger"
              :disabled="cleanupBusy"
              class="inline-flex items-center gap-1 rounded-full border border-stone-200 px-2.5 py-1 text-[10px] text-stone-700 transition hover:border-stone-300 disabled:cursor-not-allowed disabled:opacity-55"
              type="button"
              @click="requestCleanup"
            >
              <Trash2 class="h-3 w-3" />
              <span>{{ cleanupConnected ? "执行" : "入口" }}</span>
            </button>
          </div>

          <div class="flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-stone-500">
            <label
              v-for="option in STATUS_OPTIONS.filter((item) => item.value === 'expired' || item.value === 'reclaimable')"
              :key="`cleanup-${option.value}`"
              class="inline-flex items-center gap-1.5"
            >
              <input
                :checked="cleanupStatuses.includes(option.value)"
                :data-testid="`attachment-cleanup-${option.value}`"
                class="h-3.5 w-3.5 accent-stone-700"
                type="checkbox"
                @change="toggleCleanupStatus(option.value)"
              />
              <span>{{ option.label }}</span>
            </label>
          </div>

          <p
            v-if="cleanupMessage"
            :class="cleanupMessageClass(cleanupTone)"
            class="text-[11px] leading-5"
            data-testid="attachment-cleanup-message"
          >
            {{ cleanupMessage }}
          </p>
        </div>
      </div>
    </div>
  </div>
</template>
