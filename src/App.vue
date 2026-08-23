<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { TooltipProvider } from "reka-ui";
import { ChevronLeft, ChevronRight } from "lucide-vue-next";
import HomeSidebar from "@/components/HomeSidebar.vue";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import TitleBar from "@/components/TitleBar.vue";
import TelemetryPage from "@/components/telemetry/TelemetryPage.vue";
import ConfigPage from "@/components/config/ConfigPage.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import { useSettingsStore } from "@/stores/settings";
import type { ConfigTab, SidebarNavigationPage } from "@/types/config";

type AppPage = "home" | "config" | "telemetry";

const RIGHT_SIDEBAR_OPEN_STORAGE_KEY = "pony-agent.ui.right-sidebar-open";
const AUTO_CLOSE_RIGHT_SIDEBAR_WIDTH = 1000;
const AUTO_COLLAPSE_LEFT_SIDEBAR_WIDTH = 820;
const currentPage = ref<AppPage>("home");
// PA-096：配置页 tab 为会话内受控状态。两个一级键（模型配置/设置）本身是
// 显式目的地，应用启动后总从 home 开始——持久化该值只会产生只写不读的死状态。
const configTab = ref<ConfigTab>("general");
const rightSidebarPreferredOpen = ref(true);
const windowWidth = ref(typeof window !== "undefined" ? window.innerWidth : Number.POSITIVE_INFINITY);
const providerStore = useProviderStore();
const runtimeStore = useRuntimeStore();
const settingsStore = useSettingsStore();
const isResizing = ref(false);
let resizeTimer: ReturnType<typeof setTimeout> | null = null;
let onBeforeUnload: (() => void) | null = null;
let onPageHide: (() => void) | null = null;
let onVisibility: (() => void) | null = null;
let longTaskObserver: PerformanceObserver | null = null;

const forceCloseRightSidebar = computed(() => windowWidth.value < AUTO_CLOSE_RIGHT_SIDEBAR_WIDTH);
const forceCollapseLeftSidebar = computed(() => windowWidth.value < AUTO_COLLAPSE_LEFT_SIDEBAR_WIDTH);
const rightSidebarOpen = computed(() => rightSidebarPreferredOpen.value && !forceCloseRightSidebar.value);

// PA-096：左栏一级键高亮跟随派生导航态；config+tools 无对应一级键 → 不点亮。
const sessionSidebarActivePage = computed<SidebarNavigationPage | null>(() => {
  if (currentPage.value === "config") {
    if (configTab.value === "models") {
      return "models";
    }
    if (configTab.value === "general") {
      return "settings";
    }
    return null;
  }
  return currentPage.value;
});

// 左栏一级菜单请求映射：models/settings 打开配置页对应 tab，telemetry 直达遥测页。
function handleSessionNavigate(page: SidebarNavigationPage) {
  if (page === "home") {
    currentPage.value = "home";
    return;
  }

  if (page === "telemetry") {
    currentPage.value = "telemetry";
    return;
  }

  configTab.value = page === "models" ? "models" : "general";
  currentPage.value = "config";
}

function logLifecycle(event: string) {
  console.info(`[pony-agent][app] ${event}`, {
    href: window.location.href,
    ts: new Date().toISOString(),
    messages: runtimeStore.messages.length,
    traces: runtimeStore.turnTraceHistory.length,
    phase: runtimeStore.phase
  });
}

function handleWindowResize() {
  windowWidth.value = window.innerWidth;
  if (!isResizing.value) {
    isResizing.value = true;
  }
  if (resizeTimer !== null) {
    clearTimeout(resizeTimer);
    resizeTimer = null;
  }
  resizeTimer = setTimeout(() => {
    isResizing.value = false;
  }, 200);
}

async function runStartupTask(label: string, task: () => Promise<unknown>) {
  try {
    await task();
  } catch (error) {
    console.error(`[pony-agent][app] startup failed: ${label}`, {
      error: String(error)
    });
  }
}

function setupLongTaskObserver() {
  if (typeof window === "undefined" || typeof PerformanceObserver === "undefined") {
    return;
  }

  try {
    longTaskObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.duration < 120) {
          continue;
        }
        console.warn("[pony-agent][perf] long-task", {
          name: entry.name,
          duration: entry.duration,
          startTime: entry.startTime,
          page: currentPage.value,
          sessionId: runtimeStore.sessionId,
          isSubmitting: runtimeStore.isSubmitting,
          messageCount: runtimeStore.messages.length
        });
      }
    });
    longTaskObserver.observe({ entryTypes: ["longtask"] });
  } catch {
    longTaskObserver = null;
  }
}

onMounted(async () => {
  logLifecycle("mounted");
  if (typeof window !== "undefined") {
    const storedSidebarPreference = window.localStorage.getItem(RIGHT_SIDEBAR_OPEN_STORAGE_KEY);
    if (storedSidebarPreference != null) {
      rightSidebarPreferredOpen.value = storedSidebarPreference !== "false";
    }
    windowWidth.value = window.innerWidth;
  }
  onBeforeUnload = () => {
    logLifecycle("beforeunload");
    runtimeStore.persistHistory();
  };
  onPageHide = () => logLifecycle("pagehide");
  onVisibility = () => logLifecycle(`visibility:${document.visibilityState}`);

  window.addEventListener("beforeunload", onBeforeUnload);
  window.addEventListener("pagehide", onPageHide);
  document.addEventListener("visibilitychange", onVisibility);
  window.addEventListener("resize", handleWindowResize, { passive: true });
  setupLongTaskObserver();

  await Promise.all([
    runStartupTask("providerRegistry", () => providerStore.loadRegistry()),
    runStartupTask("appSettings", () => settingsStore.loadSettings()),
    runStartupTask("health", () => runtimeStore.fetchHealth()),
    runStartupTask("availableTools", () => runtimeStore.fetchAvailableTools()),
    runStartupTask("turnEvents", () => runtimeStore.initializeTurnEvents())
  ]);
  await runStartupTask("sessions", () => runtimeStore.initializeSessions());
});

onBeforeUnmount(() => {
  if (onBeforeUnload) {
    window.removeEventListener("beforeunload", onBeforeUnload);
  }
  if (onPageHide) {
    window.removeEventListener("pagehide", onPageHide);
  }
  if (onVisibility) {
    document.removeEventListener("visibilitychange", onVisibility);
  }
  window.removeEventListener("resize", handleWindowResize);
  if (resizeTimer !== null) {
    clearTimeout(resizeTimer);
    resizeTimer = null;
  }
  longTaskObserver?.disconnect();
  longTaskObserver = null;
  logLifecycle("beforeUnmount");
});

watch(rightSidebarPreferredOpen, (value) => {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(RIGHT_SIDEBAR_OPEN_STORAGE_KEY, value ? "true" : "false");
  }
});
</script>

<template>
  <TooltipProvider>
    <main
      class="flex flex-col h-screen overflow-hidden bg-transparent text-stone-900"
      :class="{ resizing: isResizing }"
    >
      <TitleBar />

      <section
        class="flex min-h-0 flex-1 w-full min-w-0 gap-4 bg-[radial-gradient(circle_at_top,rgba(248,226,184,0.10),transparent_26%),linear-gradient(180deg,#fdfbf9_0%,#faf7f2_48%,#f6f1ea_100%)] pb-3"
        data-testid="app-layout-shell"
      >
        <HomeSessionSidebar :current-page="sessionSidebarActivePage" :force-collapsed="forceCollapseLeftSidebar" @navigate="handleSessionNavigate" />

        <section class="flex flex-col min-h-0 min-w-0 flex-1">
          <Transition
            mode="out-in"
            enter-active-class="transition-all duration-300 ease-out"
            enter-from-class="translate-y-2 opacity-0"
            enter-to-class="translate-y-0 opacity-100"
            leave-active-class="transition-all duration-200 ease-in"
            leave-from-class="translate-y-0 opacity-100"
            leave-to-class="-translate-y-1 opacity-0"
          >
            <div
              v-if="currentPage === 'home'"
              key="page-home"
              :class="rightSidebarOpen ? 'gap-4' : 'gap-0'"
              class="relative flex min-h-0 min-w-0 flex-1 flex-col transition-[gap] duration-300 ease-out lg:flex-row"
              data-testid="home-layout-shell"
            >
              <div class="flex flex-col min-h-0 min-w-0 flex-1">
                <HomeWorkspace class="min-h-0 flex-1" />
              </div>
              <div
                :class="
                  rightSidebarOpen
                    ? 'max-h-[70rem] opacity-100 translate-x-0 lg:w-[20rem] lg:max-h-none xl:w-[21rem]'
                    : 'pointer-events-none max-h-0 opacity-0 translate-x-6 lg:w-0 lg:max-h-none'
                "
                class="min-h-0 min-w-0 shrink-0 overflow-hidden transition-[width,max-height,opacity,transform] duration-300 ease-out"
                :data-open="rightSidebarOpen ? 'true' : 'false'"
                data-testid="home-right-sidebar-shell"
              >
                <div class="h-full min-h-0 min-w-0 lg:w-[20rem] xl:w-[21rem]">
                  <HomeSidebar />
                </div>
              </div>
                <button
                  v-if="!forceCloseRightSidebar"
                  type="button"
                  class="absolute right-3 top-2 z-20 inline-flex h-8 w-8 items-center justify-center rounded-[0.5rem] bg-[#fbf4e8] text-stone-500 transition-[background-color,color] duration-300 ease-out hover:cursor-pointer hover:bg-[#f7e3bf] hover:text-stone-900"
                  :aria-label="rightSidebarOpen ? '隐藏右侧边栏' : '显示右侧边栏'"
                  :title="rightSidebarOpen ? '隐藏右侧边栏' : '显示右侧边栏'"
                  :data-open="rightSidebarOpen ? 'true' : 'false'"
                  data-testid="workspace-right-sidebar-toggle"
                  @click="rightSidebarPreferredOpen = !rightSidebarPreferredOpen"
                >
                <ChevronRight v-if="rightSidebarOpen" class="h-4 w-4" />
                <ChevronLeft v-else class="h-4 w-4" />
              </button>
            </div>

            <ConfigPage
              v-else-if="currentPage === 'config'"
              key="page-config"
              v-model:tab="configTab"
              class="h-full"
            />
            <TelemetryPage v-else key="page-telemetry" class="h-full" @navigate="handleSessionNavigate" />
          </Transition>
        </section>
      </section>
    </main>
  </TooltipProvider>
</template>
