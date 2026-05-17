<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import { storeToRefs } from "pinia";
import { ChevronDown, Pencil, Plus, Save, Settings2, Trash2 } from "lucide-vue-next";
import InfoTip from "@/components/InfoTip.vue";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";
import { useProviderStore } from "@/stores/providers";
import type { ProviderModelConfig } from "@/types/provider";

type ModelFormState = {
  id: string | null;
  name: string;
  model: string;
  temperature: string;
  maxOutputTokens: string;
  showAdvanced: boolean;
  mode: "create" | "edit";
};

const providerStore = useProviderStore();
const { currentProvider, error, loading, notice, providers, saving } = storeToRefs(providerStore);
const modelSaving = computed(() => saving.value);

const modelForm = reactive<ModelFormState>({
  id: null,
  name: "",
  model: "",
  temperature: "",
  maxOutputTokens: "",
  showAdvanced: false,
  mode: "create"
});

const modelFormTitle = computed(() => (modelForm.mode === "edit" ? "编辑模型" : "新增模型"));

function createId(prefix: string) {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}-${crypto.randomUUID()}`;
  }

  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
}

function resetModelForm() {
  modelForm.id = null;
  modelForm.name = "";
  modelForm.model = "";
  modelForm.temperature = "";
  modelForm.maxOutputTokens = "";
  modelForm.showAdvanced = false;
  modelForm.mode = "create";
}

function startCreateModel() {
  resetModelForm();
}

function startEditModel(model: ProviderModelConfig) {
  modelForm.id = model.id;
  modelForm.name = model.name;
  modelForm.model = model.model;
  modelForm.temperature = model.temperature > 0 ? String(model.temperature) : "";
  modelForm.maxOutputTokens = model.maxOutputTokens > 0 ? String(model.maxOutputTokens) : "";
  modelForm.showAdvanced = model.temperature > 0 || model.maxOutputTokens > 0;
  modelForm.mode = "edit";
}

async function saveModelForm() {
  if (!currentProvider.value) {
    return;
  }

  const name = modelForm.name.trim();
  const model = modelForm.model.trim();

  if (!name || !model) {
    return;
  }

  providerStore.upsertModel(currentProvider.value.id, {
    id: modelForm.id ?? createId("model"),
    name,
    model,
    temperature: modelForm.temperature.trim() ? Number(modelForm.temperature) : 0,
    maxOutputTokens: modelForm.maxOutputTokens.trim() ? Number(modelForm.maxOutputTokens) : 0
  });

  await providerStore.saveRegistry();
  if (!providerStore.error) {
    providerStore.notice = modelForm.mode === "edit" ? "模型已更新。" : "模型已新增。";
    resetModelForm();
  }
}

watch(
  () => currentProvider.value?.id,
  () => {
    resetModelForm();
  }
);
</script>

<template>
  <section class="grid min-w-0 gap-3 lg:grid-cols-[minmax(240px,0.7fr)_minmax(0,1.3fr)]">
    <aside class="min-w-0 rounded-[0.6rem] bg-white px-4 py-4">
      <div class="flex items-center justify-between gap-3">
        <div>
          <div class="flex items-center gap-2">
            <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">Provider 列表</h2>
            <InfoTip text="这里管理的是模型提供商与模型列表本身。API Key 不进入普通 JSON 长期配置，而是按提供商名称自动写到对应环境变量里。" />
          </div>
          <p class="mt-1 text-sm leading-6 text-stone-500">支持新增、编辑提供商以及每个提供商下的模型。</p>
        </div>
        <Settings2 class="h-4 w-4 text-stone-400" />
      </div>

      <div class="mt-4 flex gap-2">
        <Button size="sm" @click="providerStore.addProvider('openai')">
          <Plus class="mr-1 h-3.5 w-3.5" />
          OpenAI 协议
        </Button>
        <Button size="sm" variant="secondary" @click="providerStore.addProvider('anthropic')">
          <Plus class="mr-1 h-3.5 w-3.5" />
          Anthropic 协议
        </Button>
      </div>

      <div class="mt-4 space-y-2">
        <button
          v-for="provider in providers"
          :key="provider.id"
          type="button"
          class="w-full rounded-[0.45rem] px-3 py-3 text-left transition"
          :class="
            currentProvider?.id === provider.id
              ? 'bg-stone-900 text-amber-50'
              : 'bg-[#f3eee6] text-stone-700 hover:bg-[#ede5d9]'
          "
          @click="providerStore.selectProvider(provider.id)"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="font-medium">{{ provider.name }}</div>
            <div class="text-[11px] uppercase tracking-[0.18em]" :class="currentProvider?.id === provider.id ? 'text-amber-100/70' : 'text-stone-400'">
              {{ provider.protocol }}
            </div>
          </div>
          <div class="mt-1 text-xs" :class="currentProvider?.id === provider.id ? 'text-amber-50/75' : 'text-stone-500'">
            {{ provider.models.length }} 个模型 · {{ provider.apiKeyEnvVar }}
          </div>
        </button>
      </div>
    </aside>

    <section class="min-w-0 rounded-[0.6rem] bg-white px-4 py-4 sm:px-5">
      <div v-if="loading" class="rounded-[0.45rem] bg-amber-50/65 px-4 py-4 text-sm text-stone-500">正在读取配置...</div>

      <div v-else-if="currentProvider" class="space-y-5">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="flex items-center gap-2">
              <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">{{ currentProvider.name }}</h2>
              <InfoTip text="协议决定 Rust core 以什么请求格式调用模型。当前支持 openai 兼容协议和 anthropic messages 协议。" />
            </div>
            <p class="mt-1 text-sm leading-6 text-stone-500">普通配置会被持久化；保存 provider 时会自动写入并覆盖对应环境变量。</p>
          </div>

          <div class="flex gap-2">
            <Button variant="secondary" :disabled="saving" @click="providerStore.saveRegistry()">
              <Save class="mr-1 h-4 w-4" />
              {{ saving ? "保存中" : "保存配置" }}
            </Button>
            <Button variant="ghost" :disabled="providers.length <= 1" @click="providerStore.removeProvider(currentProvider.id)">
              <Trash2 class="mr-1 h-4 w-4" />
              删除 provider
            </Button>
          </div>
        </div>

        <div class="grid gap-3 lg:grid-cols-2">
          <div class="space-y-2 rounded-[0.45rem] bg-[#f6efe5] px-4 py-4">
            <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
              提供商信息
              <InfoTip text="提供商名称会自动推导环境变量名，例如 deepseek 会自动写入 DEEPSEEK_API_KEY。" />
            </div>

            <label class="space-y-1 text-xs text-stone-500">
              <span>提供商名称</span>
              <Input :model-value="currentProvider.name" @update:model-value="providerStore.updateProviderField(currentProvider.id, 'name', $event)" />
            </label>

            <label class="space-y-1 text-xs text-stone-500">
              <span>协议</span>
              <select
                :value="currentProvider.protocol"
                class="h-11 w-full rounded-[0.45rem] bg-white px-3 text-sm text-stone-900 outline-none"
                @change="providerStore.updateProviderField(currentProvider.id, 'protocol', ($event.target as HTMLSelectElement).value as 'openai' | 'anthropic')"
              >
                <option value="openai">openai</option>
                <option value="anthropic">anthropic</option>
              </select>
            </label>

            <label class="space-y-1 text-xs text-stone-500">
              <span>Base URL</span>
              <Input :model-value="currentProvider.baseUrl" @update:model-value="providerStore.updateProviderField(currentProvider.id, 'baseUrl', $event)" />
            </label>
          </div>

          <div class="space-y-2 rounded-[0.45rem] bg-[#f6efe5] px-4 py-4 text-stone-900">
            <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
              API Key
              <InfoTip text="这里允许直接录入和覆盖当前 provider 的 API Key。点击保存配置时，会自动写入按提供商名称推导出来的环境变量，同时当前进程也会立即可读。" />
            </div>

            <label class="space-y-1 text-xs text-stone-500">
              <span>当前密钥</span>
              <Input
                :model-value="currentProvider.apiKeyValue"
                class="bg-white text-stone-900 placeholder:text-stone-400"
                type="password"
                placeholder="输入后保存配置即可自动写入"
                @update:model-value="providerStore.updateProviderField(currentProvider.id, 'apiKeyValue', $event)"
              />
            </label>
          </div>
        </div>

        <div class="space-y-3 px-1 py-1">
          <div class="flex items-center justify-between gap-3">
            <div>
              <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                模型列表
                <InfoTip text="这里先显示已有模型，再通过下方表单新增或编辑。主页底部的模型切换条，直接读取这里保存下来的结果。" />
              </div>
              <p class="mt-1 text-xs leading-5 text-stone-500">列表负责浏览与删除；新增和编辑统一走下方表单，避免在列表里堆太多输入框。</p>
            </div>
            <Button size="sm" @click="startCreateModel()">
              <Plus class="mr-1 h-3.5 w-3.5" />
              新增模型
            </Button>
          </div>

          <div class="space-y-2">
            <div
              v-for="model in currentProvider.models"
              :key="model.id"
              class="rounded-[0.45rem] bg-[#f8f4ee] px-3 py-3"
            >
              <div class="flex flex-wrap items-center justify-between gap-3">
                <div class="min-w-0">
                  <div class="text-sm font-medium text-stone-900">{{ model.name || "未命名模型" }}</div>
                  <div class="mt-1 text-xs text-stone-500">{{ model.model || "未填写模型 ID" }}</div>
                </div>

                <div class="flex items-center gap-2">
                  <label class="inline-flex items-center gap-2 text-xs font-medium text-stone-500">
                    <input
                      type="radio"
                      name="selected-model"
                      :checked="currentProvider.selectedModelId === model.id"
                      @change="providerStore.selectModel(currentProvider.id, model.id)"
                    />
                    默认
                  </label>
                  <Button size="sm" variant="ghost" @click="startEditModel(model)">
                    <Pencil class="mr-1 h-3.5 w-3.5" />
                    编辑
                  </Button>
                  <Button size="sm" variant="ghost" @click="providerStore.removeModel(currentProvider.id, model.id)">
                    <Trash2 class="mr-1 h-3.5 w-3.5" />
                    删除
                  </Button>
                </div>
              </div>
            </div>
          </div>

          <div class="rounded-[0.45rem] bg-[#f6efe5] px-4 py-4">
            <div class="flex items-center justify-between gap-3">
              <div class="text-sm font-medium text-stone-900">{{ modelFormTitle }}</div>
              <Button v-if="modelForm.mode === 'edit'" size="sm" variant="ghost" @click="startCreateModel()">
                切换为新增
              </Button>
            </div>

            <div class="mt-3 grid gap-3 lg:grid-cols-2">
              <label class="space-y-1 text-xs text-stone-500">
                <span>名称</span>
                <Input :model-value="modelForm.name" placeholder="例如：DeepSeek Chat" @update:model-value="modelForm.name = $event" />
              </label>

              <label class="space-y-1 text-xs text-stone-500">
                <span>模型 ID</span>
                <Input :model-value="modelForm.model" placeholder="例如：deepseek-chat" @update:model-value="modelForm.model = $event" />
              </label>
            </div>

            <button
              class="mt-3 inline-flex items-center gap-1 text-[11px] text-stone-500"
              type="button"
              @click="modelForm.showAdvanced = !modelForm.showAdvanced"
            >
              <ChevronDown class="h-3.5 w-3.5 transition-transform" :class="modelForm.showAdvanced ? 'rotate-180' : ''" />
              {{ modelForm.showAdvanced ? "收起可选参数" : "展开可选参数" }}
            </button>

            <div v-if="modelForm.showAdvanced" class="mt-3 grid gap-3 lg:grid-cols-2">
              <label class="space-y-1 text-xs text-stone-500">
                <span>Temperature</span>
                <Input :model-value="modelForm.temperature" type="number" placeholder="留空则使用默认值" @update:model-value="modelForm.temperature = $event" />
              </label>

              <label class="space-y-1 text-xs text-stone-500">
                <span>Max Output Tokens</span>
                <Input :model-value="modelForm.maxOutputTokens" type="number" placeholder="留空则使用默认值" @update:model-value="modelForm.maxOutputTokens = $event" />
              </label>
            </div>

            <div class="mt-4 flex justify-end gap-2">
              <Button variant="ghost" @click="resetModelForm()">清空</Button>
              <Button :disabled="modelSaving || !modelForm.name.trim() || !modelForm.model.trim()" @click="saveModelForm()">
                <Save class="mr-1 h-4 w-4" />
                {{ modelSaving ? "保存中" : "保存" }}
              </Button>
            </div>
          </div>
        </div>

        <div v-if="notice" class="rounded-[0.45rem] bg-amber-100/70 px-4 py-3 text-sm text-amber-950">
          {{ notice }}
        </div>
        <div v-if="error" class="rounded-[0.45rem] bg-rose-50 px-4 py-3 text-sm text-rose-800">
          {{ error }}
        </div>
      </div>

      <div v-else class="rounded-[0.45rem] bg-amber-50/65 px-4 py-4 text-sm text-stone-500">
        当前没有 provider，请先新增一个。
      </div>
    </section>
  </section>
</template>
