<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import HomeSidebar from "@/components/HomeSidebar.vue";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import ProviderConfigPage from "@/components/ProviderConfigPage.vue";
import Button from "@/components/ui/Button.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";

const currentPage = ref<"home" | "providers">("home");
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

onMounted(async () => {
  logLifecycle("mounted");
  onBeforeUnload = () => logLifecycle("beforeunload");
  onPageHide = () => logLifecycle("pagehide");
  onVisibility = () => logLifecycle(`visibility:${document.visibilityState}`);

  window.addEventListener("beforeunload", onBeforeUnload);
  window.addEventListener("pagehide", onPageHide);
  document.addEventListener("visibilitychange", onVisibility);

  await Promise.all([
    providerStore.loadRegistry(),
    runtimeStore.fetchHealth(),
    runtimeStore.fetchAvailableTools(),
    runtimeStore.initializeTurnEvents()
  ]);
  await runtimeStore.initializeSessions();
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
</script>

<template>
  <main
    class="h-screen overflow-hidden bg-[radial-gradient(circle_at_top,rgba(245,218,161,0.22),transparent_24%),linear-gradient(180deg,#f7f4ee_0%,#f1ece4_48%,#ece7de_100%)] text-stone-900"
  >
    <section class="mx-auto flex h-full min-h-0 w-full max-w-[1480px] min-w-0 flex-col px-4 pt-4 pb-2 sm:px-5 lg:px-6">
      <header class="flex items-start justify-between gap-4 border-b border-stone-200/70 pb-4">
        <div class="min-w-0">
          <h1 class="text-[2rem] font-black leading-none tracking-[-0.06em] text-stone-950 sm:text-[2.4rem]">
            Pony Agent
          </h1>
          <p class="mt-1 text-[12px] tracking-[0.2em] text-stone-500 uppercase">
            Rust Agent Core
          </p>
        </div>

        <div class="inline-flex rounded-[0.55rem] border border-stone-200/80 bg-white/70 p-1">
          <Button
            class="rounded-[0.4rem] px-4"
            :variant="currentPage === 'home' ? 'default' : 'ghost'"
            size="sm"
            @click="currentPage = 'home'"
          >
            主页
          </Button>
          <Button
            class="rounded-[0.4rem] px-4"
            :variant="currentPage === 'providers' ? 'default' : 'ghost'"
            size="sm"
            @click="currentPage = 'providers'"
          >
            模型配置
          </Button>
        </div>
      </header>

      <section class="min-h-0 min-w-0 flex-1 pt-4 pb-2">
        <div
          v-if="currentPage === 'home'"
          class="flex h-full min-h-0 min-w-0 flex-col gap-4 lg:flex-row"
        >
          <HomeSessionSidebar />
          <div class="min-h-0 min-w-0 flex-1">
            <HomeWorkspace />
          </div>
          <div class="min-h-0 min-w-0 shrink-0 lg:w-[20rem] xl:w-[21rem]">
            <HomeSidebar />
          </div>
        </div>
        <ProviderConfigPage v-else />
      </section>
    </section>
  </main>
</template>
