<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { Check, ChevronRight, ExternalLink, LoaderCircle, Pencil, Save, Settings2, Shield } from "lucide-vue-next";
import { useSettingsStore } from "@/stores/settings";
import Input from "@/components/ui/Input.vue";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";

const settingsStore = useSettingsStore();
const { settings, saving } = storeToRefs(settingsStore);
const isCoding = computed(() => settings.value.workspaceMode === "coding");

function chooseMode(mode: "coding" | "work") {
  void settingsStore.setWorkspaceMode(mode);
}

const exaKey = ref("");
const exaSaving = ref(false);
const exaKeyPresent = ref(false);
const exaEditing = ref(false);

onMounted(async () => {
  if (!isTauriAvailable()) return;
  try {
    const key = await safeInvoke<string>("get_service_api_key", { service: "exa" });
    exaKey.value = key;
    exaKeyPresent.value = !!key;
  } catch {
    // browser mode
  }
});

function beginEditExa() {
  exaEditing.value = true;
}

function cancelEditExa() {
  exaEditing.value = false;
  if (exaKeyPresent.value) {
    safeInvoke<string>("get_service_api_key", { service: "exa" }).then(k => {
      exaKey.value = k;
    });
  } else {
    exaKey.value = "";
  }
}

async function saveExaKey() {
  if (!isTauriAvailable()) return;
  exaSaving.value = true;
  try {
    await safeInvoke("set_service_api_key", { service: "exa", key: exaKey.value });
    exaKeyPresent.value = !!exaKey.value;
    exaEditing.value = false;
  } catch {
    // ignore
  } finally {
    exaSaving.value = false;
  }
}

function openExa() {
  if (isTauriAvailable()) {
    safeInvoke("open_url", { url: "https://exa.ai/" });
  } else {
    window.open("https://exa.ai/", "_blank");
  }
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

    <div class="flex min-h-0 flex-1 flex-col gap-6 px-4 py-4">
      <div class="space-y-2">
        <div class="text-[12px] font-medium text-stone-500">工作模式</div>
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

      <div class="space-y-2">
        <div class="text-[12px] font-medium text-stone-500">服务密钥</div>
        <div class="flex items-center gap-2 w-full">
          <div class="flex items-center gap-1.5 text-[13px] font-medium text-stone-900 shrink-0">
            EXA 密钥
            <Shield class="h-3.5 w-3.5 text-stone-500" />
          </div>
          <div class="relative flex-1 max-w-[480px]">
            <Input
              :model-value="exaKey"
              type="password"
              :disabled="!exaEditing || exaSaving"
              placeholder="输入 Exa API Key"
              class="bg-stone-100/80 text-[9px] tracking-[-0.03em]"
              @update:model-value="exaKey = $event"
            />
            <div class="absolute right-1 top-1/2 -translate-y-1/2 flex items-center gap-0.5">
              <template v-if="exaEditing">
                <button
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700 disabled:pointer-events-none disabled:opacity-50"
                  :disabled="exaSaving"
                  @click="saveExaKey()"
                >
                  <LoaderCircle v-if="exaSaving" class="h-3.5 w-3.5 animate-spin" />
                  <Save v-else class="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700"
                  @click="cancelEditExa()"
                >
                  <span class="text-[10px]">✕</span>
                </button>
              </template>
              <template v-else>
                <button
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700"
                  @click="beginEditExa()"
                >
                  <Pencil class="h-3.5 w-3.5" />
                </button>
              </template>
            </div>
          </div>
          <Check
            v-if="exaKeyPresent && !exaEditing"
            class="h-4 w-4 shrink-0 text-emerald-600"
          />
          <button
            type="button"
            class="inline-flex shrink-0 items-center gap-0.5 text-[11px] text-stone-500 hover:text-stone-800"
            @click="openExa()"
          >
            exa.ai
            <ExternalLink class="h-3 w-3" />
          </button>
        </div>
      </div>
    </div>
  </section>
</template>
