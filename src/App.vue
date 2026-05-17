<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import HomeSidebar from "@/components/HomeSidebar.vue";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import InfoTip from "@/components/InfoTip.vue";
import ProviderConfigPage from "@/components/ProviderConfigPage.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";

const currentPage = ref<"home" | "providers">("home");
const providerStore = useProviderStore();
const runtimeStore = useRuntimeStore();

const pageTitle = computed(() =>
  currentPage.value === "home" ? "对话工作台" : "模型与提供商配置"
);

onMounted(async () => {
  await Promise.all([providerStore.loadRegistry(), runtimeStore.fetchHealth()]);
});
</script>

<template>
  <main
    class="min-h-screen overflow-x-hidden bg-[radial-gradient(circle_at_top,rgba(245,218,161,0.28),transparent_26%),linear-gradient(180deg,#f5f0e7_0%,#ece6dc_42%,#f3ede4_100%)] text-stone-900"
  >
    <section class="mx-auto flex min-h-screen w-full max-w-[1500px] min-w-0 flex-col px-3 py-3 sm:px-4 sm:py-4 lg:px-6">
      <header class="grid gap-3 px-1 py-2 md:grid-cols-[1.1fr_0.9fr] md:px-2">
        <div class="space-y-3">
          <div class="flex flex-wrap items-center gap-2">
            <Badge variant="default">Pony Agent</Badge>
            <Badge>Rust Agent Core</Badge>
            <Badge variant="success">跨平台工作台</Badge>
          </div>
          <div class="space-y-2">
            <div class="flex items-center gap-2">
              <h1 class="text-[1.55rem] font-semibold tracking-[-0.03em] text-stone-950 sm:text-[1.8rem]">
                {{ pageTitle }}
              </h1>
              <InfoTip text="前端是工作台，真正的 agent core 在 Rust 后端。后续即使替换桌面壳层，核心也应保留为可独立部署的引擎。" />
            </div>
            <p class="max-w-3xl text-sm leading-6 text-stone-600 sm:text-[15px]">
              当前这版界面优先服务调试、学习和配置，不追求厚重面板感。区域通过背景和排版自然分层，避免靠边框切碎视线。
            </p>
          </div>
        </div>

        <div class="flex flex-col justify-between gap-4 md:items-end">
          <div class="inline-flex w-full rounded-[0.65rem] bg-white/55 p-1 md:w-auto">
            <Button
              class="flex-1 rounded-[0.5rem] px-4 sm:px-5"
              :variant="currentPage === 'home' ? 'default' : 'ghost'"
              size="sm"
              @click="currentPage = 'home'"
            >
              主页
            </Button>
            <Button
              class="flex-1 rounded-[0.5rem] px-4 sm:px-5"
              :variant="currentPage === 'providers' ? 'default' : 'ghost'"
              size="sm"
              @click="currentPage = 'providers'"
            >
              模型配置
            </Button>
          </div>

          <div class="space-y-1 text-xs leading-5 text-stone-500 md:text-right">
            <div>工作台层：Vue + Tauri</div>
            <div>引擎层：Rust runtime / provider / session / tools</div>
          </div>
        </div>
      </header>

      <section class="mt-3 min-w-0 flex-1">
        <div v-if="currentPage === 'home'" class="grid h-full min-w-0 gap-3 lg:grid-cols-[minmax(0,1.38fr)_minmax(300px,0.62fr)]">
          <HomeWorkspace />
          <HomeSidebar />
        </div>
        <ProviderConfigPage v-else />
      </section>
    </section>
  </main>
</template>
