<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Settings2 } from "lucide-vue-next";
import ProviderConfigPage from "@/components/ProviderConfigPage.vue";
import ConfigGeneralSection from "@/components/config/ConfigGeneralSection.vue";
import ConfigToolsSection from "@/components/config/ConfigToolsSection.vue";
import type { ConfigTab } from "@/types/config";

/**
 * PA-096：配置页 tab 容器（通用 / 模型 / 工具）。
 * - tab 为受控态：激活 tab 由 App 的 configTab ref 驱动、会话内不持久化
 *   （PA-096 实现修订：一级键即显式目的地，持久化只会产生只写不读的死状态）；
 * - APG tabs：roving tabindex + 方向键 + aria-controls/tabpanel 配对；
 * - body v-if 懒挂载，models tab 直接承载 ProviderConfigPage（grid h-full 填满）。
 */

const props = defineProps<{
  tab: ConfigTab;
}>();

const emit = defineEmits<{
  (event: "update:tab", tab: ConfigTab): void;
}>();

const CONFIG_TABS: Array<{ id: ConfigTab; label: string; description: string }> = [
  { id: "general", label: "通用", description: "工作模式与服务密钥" },
  { id: "models", label: "模型", description: "提供商接入与模型挂载" },
  { id: "tools", label: "工具", description: "可用工具目录与权限摘要" }
];

const activeTab = computed(() => (CONFIG_TABS.some((item) => item.id === props.tab) ? props.tab : "general"));

const headingRef = ref<HTMLElement | null>(null);
const tabRefs = ref<HTMLButtonElement[]>([]);

function setTabRef(element: unknown, index: number) {
  if (element instanceof HTMLButtonElement) {
    tabRefs.value[index] = element;
  }
}

function selectTab(tabId: ConfigTab) {
  if (tabId !== activeTab.value) {
    emit("update:tab", tabId);
  }
}

function focusTab(index: number) {
  const nextIndex = ((index % CONFIG_TABS.length) + CONFIG_TABS.length) % CONFIG_TABS.length;
  const target = CONFIG_TABS[nextIndex]!.id;
  if (target !== props.tab) {
    emit("update:tab", target);
  }
  // tab 按钮在头部恒挂载，程序化 focus 同步即可（与 TelemetryPage 同一模式）。
  tabRefs.value[nextIndex]?.focus();
}

function handleTablistKeydown(event: KeyboardEvent) {
  const currentIndex = CONFIG_TABS.findIndex((item) => item.id === activeTab.value);
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
      focusTab(CONFIG_TABS.length - 1);
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
    data-testid="config-page"
  >
    <div class="flex shrink-0 items-center justify-between gap-3 border-b border-stone-200/70 px-4 py-3">
      <div class="flex min-w-0 items-center gap-2">
        <Settings2 class="h-4 w-4 shrink-0 text-stone-500" />
        <h2
          ref="headingRef"
          class="truncate text-sm font-semibold tracking-[-0.02em] text-stone-950 outline-none"
          tabindex="-1"
          data-testid="config-heading"
        >
          配置
        </h2>
        <span class="hidden text-[11px] leading-5 text-stone-500 sm:inline">工作模式、模型接入与工具目录</span>
      </div>

      <div
        class="flex items-center gap-1 rounded-[0.5rem] bg-[#f6f0e8] p-1"
        role="tablist"
        aria-label="配置分区"
        data-testid="config-tablist"
        @keydown="handleTablistKeydown"
      >
        <button
          v-for="(item, index) in CONFIG_TABS"
          :id="`config-tab-${item.id}`"
          :key="item.id"
          :ref="(element) => setTabRef(element, index)"
          type="button"
          role="tab"
          :aria-selected="activeTab === item.id"
          :aria-controls="`config-panel-${item.id}`"
          :title="item.description"
          :tabindex="activeTab === item.id ? 0 : -1"
          class="rounded-[0.35rem] px-3 py-1.5 text-[12px] font-medium transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
          :class="
            activeTab === item.id
              ? 'bg-white text-stone-900 shadow-[0_1px_2px_rgba(28,25,23,0.08)]'
              : 'text-stone-500 hover:text-stone-900'
          "
          :data-testid="`config-tab-${item.id}`"
          @click="selectTab(item.id)"
        >
          {{ item.label }}
        </button>
      </div>
    </div>

    <div class="min-h-0 flex-1 p-3">
      <div
        v-if="activeTab === 'general'"
        id="config-panel-general"
        role="tabpanel"
        aria-labelledby="config-tab-general"
        class="h-full min-h-0"
        data-testid="config-panel-general"
      >
        <ConfigGeneralSection />
      </div>

      <div
        v-else-if="activeTab === 'models'"
        id="config-panel-models"
        role="tabpanel"
        aria-labelledby="config-tab-models"
        class="h-full min-h-0"
        data-testid="config-panel-models"
      >
        <ProviderConfigPage class="h-full" />
      </div>

      <div
        v-else
        id="config-panel-tools"
        role="tabpanel"
        aria-labelledby="config-tab-tools"
        class="h-full min-h-0"
        data-testid="config-panel-tools"
      >
        <ConfigToolsSection />
      </div>
    </div>
  </section>
</template>
