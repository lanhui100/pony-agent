<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { PanelRightClose, PanelRightOpen } from "lucide-vue-next";
import HomeSidebar from "@/components/HomeSidebar.vue";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import ModelMonitorPage from "@/components/ModelMonitorPage.vue";
import ProviderConfigPage from "@/components/ProviderConfigPage.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";

type AppPage = "home" | "providers" | "model-monitor";

const RIGHT_SIDEBAR_OPEN_STORAGE_KEY = "pony-agent.ui.right-sidebar-open";
const currentPage = ref<AppPage>("home");
const rightSidebarOpen = ref(true);
const providerStore = useProviderStore();
const runtimeStore = useRuntimeStore();
let onBeforeUnload: (() => void) | null = null;
let onPageHide: (() => void) | null = null;
let onVisibility: (() => void) | null = null;

function logLifecycle(event: string) {
  console.info(`[pony-agent][app] ${event}`, {
    href: window.location.href,
    ts: new Date().toISOString(),
    messages: runtimeStore.messages.length,
    traces: runtimeStore.turnTraceHistory.length,
    phase: runtimeStore.phase
  });
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

onMounted(async () => {
  logLifecycle("mounted");
  if (typeof window !== "undefined") {
    const storedSidebarPreference = window.localStorage.getItem(RIGHT_SIDEBAR_OPEN_STORAGE_KEY);
    if (storedSidebarPreference != null) {
      rightSidebarOpen.value = storedSidebarPreference !== "false";
    }
  }
  onBeforeUnload = () => logLifecycle("beforeunload");
  onPageHide = () => logLifecycle("pagehide");
  onVisibility = () => logLifecycle(`visibility:${document.visibilityState}`);

  window.addEventListener("beforeunload", onBeforeUnload);
  window.addEventListener("pagehide", onPageHide);
  document.addEventListener("visibilitychange", onVisibility);

  await Promise.all([
    runStartupTask("providerRegistry", () => providerStore.loadRegistry()),
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
  logLifecycle("beforeUnmount");
});

watch(rightSidebarOpen, (value) => {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(RIGHT_SIDEBAR_OPEN_STORAGE_KEY, value ? "true" : "false");
  }
});
</script>

<template>
  <main
    class="h-screen overflow-hidden bg-[radial-gradient(circle_at_top,rgba(248,226,184,0.18),transparent_26%),linear-gradient(180deg,#fbf8f3_0%,#f6f1ea_48%,#f1ece4_100%)] text-stone-900"
  >
    <section class="mx-auto flex h-full min-h-0 w-full max-w-[1540px] min-w-0 gap-4 px-3 py-3 sm:px-4 lg:px-5">
      <HomeSessionSidebar :current-page="currentPage" @navigate="currentPage = $event" />

      <section class="min-h-0 min-w-0 flex-1">
        <div
          v-if="currentPage === 'home'"
          :class="rightSidebarOpen ? 'gap-4' : 'gap-0'"
          class="relative flex h-full min-h-0 min-w-0 flex-col transition-[gap] duration-300 ease-out lg:flex-row"
          data-testid="home-layout-shell"
        >
          <button
            type="button"
            :class="
              rightSidebarOpen
                ? 'right-3 lg:right-[calc(20rem+1rem+0.5rem)] xl:right-[calc(21rem+1rem+0.5rem)]'
                : 'right-3'
            "
            class="absolute top-2 z-20 inline-flex h-8 w-8 items-center justify-center rounded-[0.5rem] bg-[#fbf4e8] text-stone-500 transition-[right,background-color,color] duration-300 ease-out hover:bg-[#f7e3bf] hover:text-stone-900"
            :aria-label="rightSidebarOpen ? '隐藏右侧边栏' : '显示右侧边栏'"
            :title="rightSidebarOpen ? '隐藏右侧边栏' : '显示右侧边栏'"
            :data-open="rightSidebarOpen ? 'true' : 'false'"
            data-testid="workspace-right-sidebar-toggle"
            @click="rightSidebarOpen = !rightSidebarOpen"
          >
            <PanelRightClose v-if="rightSidebarOpen" class="h-4 w-4" />
            <PanelRightOpen v-else class="h-4 w-4" />
          </button>
          <div class="min-h-0 min-w-0 flex-1">
            <HomeWorkspace />
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
        </div>

        <ProviderConfigPage v-else-if="currentPage === 'providers'" class="h-full" />
        <ModelMonitorPage v-else class="h-full" />
      </section>
    </section>
  </main>
</template>
