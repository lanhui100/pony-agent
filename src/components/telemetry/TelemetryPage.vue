<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { TooltipProvider } from "reka-ui";
import { ArrowLeft, CheckCircle2, LoaderCircle, UploadCloud, XCircle } from "lucide-vue-next";
import ModelMonitorPage from "@/components/ModelMonitorPage.vue";
import TraceInspector from "@/components/TraceInspector.vue";
import Tooltip from "@/components/ui/Tooltip.vue";
import { useSettingsStore } from "@/stores/settings";
import { useRuntimeStore } from "@/stores/runtime";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";

/**
 * PA-096：二级观测页（原"遥测"，ADR 0013 更名）。
 *
 * - coding 模式：[Trace, 指标] 双 tab，默认 Trace；
 *   work 模式：仅 [指标] 单 tab（原始 turn 调试明细不可达——用户决策 2026-08-22）。
 * - 入口为对话页右栏右上角浮动图标按钮（ADR 0013），页内保留返回按钮回 home。
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

// Trace 一键上传至 PonySentry（纯手动触发，上传当前会话全部持久化 trace）
const runtimeStore = useRuntimeStore();
const uploadState = ref<"idle" | "uploading" | "done" | "error">("idle");
const uploadFeedback = ref("");
const toastMessage = ref<string | null>(null);
const toastType = ref<"success" | "error" | "info">("info");
let uploadTimer: ReturnType<typeof setTimeout> | null = null;
let toastTimer: ReturnType<typeof setTimeout> | null = null;

function showToast(message: string, type: "success" | "error" | "info" = "info", duration = 4000) {
  toastMessage.value = message;
  toastType.value = type;
  if (toastTimer) {
    clearTimeout(toastTimer);
  }
  toastTimer = setTimeout(() => {
    toastMessage.value = null;
    toastTimer = null;
  }, duration);
}

async function uploadSessionTrace() {
  if (uploadState.value === "uploading") {
    return;
  }
  if (!isTauriAvailable()) {
    showToast("当前环境不支持与本地客户端通信", "error");
    return;
  }
  if (!runtimeStore.sessionId?.trim()) {
    showToast("未找到当前活跃会话 ID", "error");
    return;
  }

  uploadState.value = "uploading";
  uploadFeedback.value = "上传中…";

  try {
    const count = await safeInvoke<number>("upload_session_trace", {
      sessionId: runtimeStore.sessionId
    });

    if (count === 0) {
      uploadState.value = "done";
      uploadFeedback.value = "本会话暂无 Trace 数据";
      showToast("当前会话暂无已完成的持久化 Trace 数据可上传", "info");
    } else {
      uploadState.value = "done";
      uploadFeedback.value = `已成功上传该会话 Trace（共 ${count} 轮对话）`;
      showToast(`已成功上传会话 Trace（包含 ${count} 轮对话链路）至 PonySentry`, "success");
    }
  } catch (err) {
    uploadState.value = "error";
    const errMsg = String(err);
    uploadFeedback.value = `上传失败: ${errMsg}`;
    showToast(`上传 Trace 失败: ${errMsg}`, "error", 5000);
  }

  if (uploadTimer) {
    clearTimeout(uploadTimer);
  }
  uploadTimer = setTimeout(() => {
    uploadState.value = "idle";
    uploadFeedback.value = "";
  }, 4000);
}

onMounted(() => {
  headingRef.value?.focus();
});

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
          观测
        </h2>
        <span v-if="isCoding" class="hidden text-[11px] leading-5 text-stone-500 sm:inline">
          Trace 与指标的二级读面
        </span>
        <span v-else class="hidden text-[11px] leading-5 text-stone-500 sm:inline">
          模型调用与延迟聚合读面
        </span>
      </div>

      <div class="flex items-center gap-2">
        <div
          v-if="uploadFeedback"
          class="text-[11px] leading-5 text-stone-500"
          data-testid="telemetry-upload-feedback"
        >
          {{ uploadFeedback }}
        </div>

        <TooltipProvider :delay-duration="200">
          <Tooltip
            v-if="isCoding && activeTab === 'trace'"
            :text="uploadState === 'uploading' ? '上传中…' : '上传本会话 Trace 至 PonySentry'"
            side="bottom"
          >
            <button
              type="button"
              class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[0.35rem] text-stone-500 transition hover:bg-[#f7e3bf] hover:text-stone-900 disabled:cursor-not-allowed disabled:opacity-50"
              aria-label="上传本会话 Trace 至 PonySentry"
              data-testid="telemetry-upload-trace"
              :disabled="uploadState === 'uploading'"
              @click="uploadSessionTrace"
            >
              <LoaderCircle
                v-if="uploadState === 'uploading'"
                class="h-4 w-4 animate-spin text-amber-600"
              />
              <UploadCloud
                v-else
                class="h-4 w-4"
              />
            </button>
          </Tooltip>
        </TooltipProvider>

        <div
          class="flex items-center gap-1 rounded-[0.5rem] bg-[#f6f0e8] p-1"
          role="tablist"
          aria-label="观测视图切换"
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

    <!-- 浮动 Toast 反馈：为用户操作提供明确的状态、成功与错误感知 -->
    <transition
      enter-active-class="transition duration-200 ease-out"
      enter-from-class="translate-y-2 opacity-0"
      enter-to-class="translate-y-0 opacity-100"
      leave-active-class="transition duration-150 ease-in"
      leave-from-class="translate-y-0 opacity-100"
      leave-to-class="translate-y-2 opacity-0"
    >
      <div
        v-if="toastMessage"
        class="fixed bottom-6 right-6 z-50 flex items-center gap-2 rounded-[0.45rem] px-3.5 py-2.5 text-xs text-white shadow-lg backdrop-blur"
        :class="{
          'bg-stone-900/90': toastType === 'info',
          'bg-emerald-900/90 border border-emerald-600/40': toastType === 'success',
          'bg-rose-900/90 border border-rose-600/40': toastType === 'error'
        }"
        data-testid="telemetry-toast"
      >
        <CheckCircle2 v-if="toastType === 'success'" class="h-4 w-4 text-emerald-400 shrink-0" />
        <XCircle v-else-if="toastType === 'error'" class="h-4 w-4 text-rose-400 shrink-0" />
        <span>{{ toastMessage }}</span>
      </div>
    </transition>
  </section>
</template>
