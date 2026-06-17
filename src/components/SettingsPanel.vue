<script setup lang="ts">
import { computed } from "vue";
import { storeToRefs } from "pinia";
import { Check, ChevronRight, LoaderCircle, Settings2 } from "lucide-vue-next";
import { useSettingsStore } from "@/stores/settings";
import Button from "@/components/ui/Button.vue";

const settingsStore = useSettingsStore();
const { settings, loading, saving, error, notice } = storeToRefs(settingsStore);
const isCoding = computed(() => settings.value.workspaceMode === "coding");

function chooseMode(mode: "coding" | "work") {
  void settingsStore.setWorkspaceMode(mode);
}
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col rounded-[0.6rem] border border-stone-200/70 bg-white/72">
    <div class="flex items-center justify-between border-b border-stone-200/70 px-4 py-3">
      <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
        <Settings2 class="h-4 w-4 text-stone-500" />
        <span>配置</span>
      </div>
      <span class="text-[11px] text-stone-500">可扩展全局设置</span>
    </div>

    <div class="flex min-h-0 flex-1 flex-col gap-4 px-4 py-4">
      <div class="space-y-2">
        <div class="text-[12px] font-medium text-stone-800">工作模式</div>
        <div class="grid grid-cols-2 gap-2 max-w-[680px]">
          <button
            type="button"
            class="flex items-start gap-3 rounded-[0.5rem] border px-3 py-3 text-left transition"
            :class="isCoding ? 'border-stone-900 bg-stone-900 text-amber-50' : 'border-stone-200 bg-white text-stone-800 hover:border-stone-300 hover:bg-stone-50'"
            :disabled="saving"
            @click="chooseMode('coding')"
          >
            <Check v-if="isCoding" class="mt-0.5 h-4 w-4" />
            <ChevronRight v-else class="mt-0.5 h-4 w-4 text-stone-400" />
            <div>
              <div class="text-sm font-medium">Coding</div>
              <div class="text-[12px] opacity-80">代码、调试、实现、测试</div>
            </div>
          </button>
          <button
            type="button"
            class="flex items-start gap-3 rounded-[0.5rem] border px-3 py-3 text-left transition"
            :class="!isCoding ? 'border-stone-900 bg-stone-900 text-amber-50' : 'border-stone-200 bg-white text-stone-800 hover:border-stone-300 hover:bg-stone-50'"
            :disabled="saving"
            @click="chooseMode('work')"
          >
            <Check v-if="!isCoding" class="mt-0.5 h-4 w-4" />
            <ChevronRight v-else class="mt-0.5 h-4 w-4 text-stone-400" />
            <div>
              <div class="text-sm font-medium">Work</div>
              <div class="text-[12px] opacity-80">写作、分析、规划、文档</div>
            </div>
          </button>
        </div>
      </div>

      <div v-if="loading || saving || error || notice" class="rounded-[0.5rem] border border-stone-200 bg-stone-50 px-3 py-2 text-[12px] text-stone-600">
        <span v-if="loading" class="inline-flex items-center gap-2"><LoaderCircle class="h-3.5 w-3.5 animate-spin" />正在加载设置…</span>
        <span v-else-if="saving" class="inline-flex items-center gap-2"><LoaderCircle class="h-3.5 w-3.5 animate-spin" />正在保存设置…</span>
        <span v-else-if="error">{{ error }}</span>
        <span v-else>{{ notice }}</span>
      </div>

      <Button variant="outline" class="mt-auto w-full justify-center" :disabled="saving" @click="settingsStore.saveSettings()">
        保存设置
      </Button>
    </div>
  </section>
</template>
