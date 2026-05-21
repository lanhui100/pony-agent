<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { ChevronDown, Pencil, Plus, Save, Shield, Trash2 } from "lucide-vue-next";
import InfoTip from "@/components/InfoTip.vue";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import { useProviderStore } from "@/stores/providers";
import type {
  ProviderConfig,
  ProviderModelConfig,
  ProviderProtocol,
  ProviderReasoningEffort
} from "@/types/provider";

type ProviderFormState = {
  name: string;
  protocol: ProviderProtocol;
  baseUrl: string;
  apiKeyValue: string;
};

type ModelFormState = {
  id: string | null;
  name: string;
  model: string;
  temperature: string;
  maxOutputTokens: string;
  contextWindowTokens: string;
  reasoningEffort: ProviderReasoningEffort | "";
  reasoningBudgetTokens: string;
  supportsTools: boolean;
  supportsStreaming: boolean;
  supportsImageInput: boolean;
  supportsReasoning: boolean;
  showAdvanced: boolean;
};

type EditorState = {
  entity: "provider" | "model";
  mode: "create" | "edit";
  providerId: string | null;
  modelId: string | null;
};

const providerStore = useProviderStore();
const { currentProvider, error, loading, notice, providers, saving } = storeToRefs(providerStore);

const openProviderId = ref<string | null>(null);
const hasInitializedEditor = ref(false);
const editorState = reactive<EditorState>({
  entity: "provider",
  mode: "create",
  providerId: null,
  modelId: null
});

const providerForm = reactive<ProviderFormState>({
  name: "",
  protocol: "openai",
  baseUrl: "https://api.openai.com/v1",
  apiKeyValue: ""
});

const modelForm = reactive<ModelFormState>({
  id: null,
  name: "",
  model: "",
  temperature: "",
  maxOutputTokens: "",
  contextWindowTokens: "",
  reasoningEffort: "",
  reasoningBudgetTokens: "",
  supportsTools: true,
  supportsStreaming: true,
  supportsImageInput: false,
  supportsReasoning: false,
  showAdvanced: false
});

const editorTitle = computed(() => {
  if (editorState.entity === "provider") {
    return editorState.mode === "edit" ? "编辑提供商" : "新增提供商";
  }

  return editorState.mode === "edit" ? "缂栬緫妯″瀷" : "鏂板妯″瀷";
});

const editorDescription = computed(() => {
  if (editorState.entity === "provider") {
    return editorState.mode === "edit"
      ? "维护提供商协议、Base URL 和 API Key。"
      : "先创建提供商，再为它添加一个或多个模型。";
  }

  return editorState.mode === "edit"
    ? "这里只维护当前模型参数，左侧列表只负责导航。"
    : "模型会挂载到当前选中的提供商下。";
});

const activeProvider = computed(() => {
  if (!editorState.providerId) {
    return null;
  }

  return providers.value.find((provider) => provider.id === editorState.providerId) ?? null;
});

const modelParentProvider = computed(() => {
  if (editorState.entity !== "model" || !editorState.providerId) {
    return null;
  }

  return providers.value.find((provider) => provider.id === editorState.providerId) ?? null;
});

function createId(prefix: string) {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}-${crypto.randomUUID()}`;
  }

  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
}

function defaultBaseUrlFor(protocol: ProviderProtocol) {
  return protocol === "anthropic" ? "https://api.anthropic.com/v1" : "https://api.openai.com/v1";
}

function findProvider(providerId: string | null) {
  if (!providerId) {
    return null;
  }

  return providers.value.find((provider) => provider.id === providerId) ?? null;
}

function findModel(providerId: string | null, modelId: string | null) {
  const provider = findProvider(providerId);
  if (!provider || !modelId) {
    return null;
  }

  return provider.models.find((model) => model.id === modelId) ?? null;
}

function resetProviderForm() {
  providerForm.name = "";
  providerForm.protocol = "openai";
  providerForm.baseUrl = defaultBaseUrlFor("openai");
  providerForm.apiKeyValue = "";
}

function fillProviderForm(provider: ProviderConfig) {
  providerForm.name = provider.name;
  providerForm.protocol = provider.protocol;
  providerForm.baseUrl = provider.baseUrl;
  providerForm.apiKeyValue = provider.apiKeyValue;
}

function resetModelForm() {
  modelForm.id = null;
  modelForm.name = "";
  modelForm.model = "";
  modelForm.temperature = "";
  modelForm.maxOutputTokens = "";
  modelForm.contextWindowTokens = "";
  modelForm.reasoningEffort = "";
  modelForm.reasoningBudgetTokens = "";
  modelForm.supportsTools = true;
  modelForm.supportsStreaming = true;
  modelForm.supportsImageInput = false;
  modelForm.supportsReasoning = false;
  modelForm.showAdvanced = false;
}

function fillModelForm(model: ProviderModelConfig) {
  modelForm.id = model.id;
  modelForm.name = model.name;
  modelForm.model = model.model;
  modelForm.temperature = model.temperature > 0 ? String(model.temperature) : "";
  modelForm.maxOutputTokens = model.maxOutputTokens > 0 ? String(model.maxOutputTokens) : "";
  modelForm.contextWindowTokens = model.capabilities.contextWindowTokens
    ? String(model.capabilities.contextWindowTokens)
    : "";
  modelForm.reasoningEffort = model.reasoningEffort ?? "";
  modelForm.reasoningBudgetTokens = model.reasoningBudgetTokens ? String(model.reasoningBudgetTokens) : "";
  modelForm.supportsTools = model.capabilities.supportsTools;
  modelForm.supportsStreaming = model.capabilities.supportsStreaming;
  modelForm.supportsImageInput = model.capabilities.supportsImageInput;
  modelForm.supportsReasoning = model.capabilities.supportsReasoning;
  modelForm.showAdvanced =
    model.temperature > 0 ||
    model.maxOutputTokens > 0 ||
    !!model.capabilities.contextWindowTokens ||
    !!model.reasoningEffort ||
    !!model.reasoningBudgetTokens ||
    !model.capabilities.supportsStreaming ||
    !model.capabilities.supportsTools ||
    model.capabilities.supportsImageInput ||
    model.capabilities.supportsReasoning;
}

function toggleProvider(providerId: string) {
  openProviderId.value = openProviderId.value === providerId ? null : providerId;
  providerStore.selectProvider(providerId);
}

function beginCreateProvider() {
  resetProviderForm();
  editorState.entity = "provider";
  editorState.mode = "create";
  editorState.providerId = null;
  editorState.modelId = null;
}

function beginEditProvider(providerId: string) {
  const provider = findProvider(providerId);
  if (!provider) {
    return;
  }

  providerStore.selectProvider(providerId);
  openProviderId.value = providerId;
  fillProviderForm(provider);
  editorState.entity = "provider";
  editorState.mode = "edit";
  editorState.providerId = providerId;
  editorState.modelId = null;
}

function beginCreateModel(providerId: string) {
  const provider = findProvider(providerId);
  if (!provider) {
    return;
  }

  providerStore.selectProvider(providerId);
  openProviderId.value = providerId;
  resetModelForm();
  editorState.entity = "model";
  editorState.mode = "create";
  editorState.providerId = providerId;
  editorState.modelId = null;
}

function beginEditModel(providerId: string, modelId: string) {
  const provider = findProvider(providerId);
  const model = findModel(providerId, modelId);
  if (!provider || !model) {
    return;
  }

  providerStore.selectProvider(providerId);
  openProviderId.value = providerId;
  fillModelForm(model);
  editorState.entity = "model";
  editorState.mode = "edit";
  editorState.providerId = providerId;
  editorState.modelId = modelId;
}

async function saveProviderForm() {
  const name = providerForm.name.trim();
  const baseUrl = providerForm.baseUrl.trim();

  if (!name || !baseUrl) {
    return;
  }

  let providerId = editorState.providerId;

  if (editorState.mode === "create") {
    providerId = providerStore.addProvider() ?? null;
  }

  if (!providerId) {
    return;
  }

  providerStore.updateProviderField(providerId, "name", name);
  providerStore.updateProviderField(providerId, "protocol", providerForm.protocol);
  providerStore.updateProviderField(providerId, "baseUrl", baseUrl);
  providerStore.updateProviderField(providerId, "apiKeyValue", providerForm.apiKeyValue.trim());

  await providerStore.saveRegistry();

  if (!providerStore.error) {
    providerStore.notice = editorState.mode === "edit" ? "提供商已更新。" : "提供商已新增。";
    beginEditProvider(providerId);
  }
}

async function removeCurrentProvider() {
  if (!editorState.providerId) {
    return;
  }

  const removingId = editorState.providerId;
  providerStore.removeProvider(removingId);
  await providerStore.saveRegistry();

  if (providerStore.error) {
    return;
  }

  providerStore.notice = "提供商已删除。";

  const nextProvider = providers.value[0] ?? null;
  if (nextProvider) {
    beginEditProvider(nextProvider.id);
  } else {
    beginCreateProvider();
  }
}

async function saveModelForm() {
  if (!editorState.providerId) {
    return;
  }

  const name = modelForm.name.trim();
  const modelIdValue = modelForm.model.trim();

  if (!name || !modelIdValue) {
    return;
  }

  const payloadId = editorState.mode === "edit" && modelForm.id ? modelForm.id : createId("model");

  providerStore.upsertModel(editorState.providerId, {
    id: payloadId,
    name,
    model: modelIdValue,
    temperature: modelForm.temperature.trim() ? Number(modelForm.temperature) : 0,
    maxOutputTokens: modelForm.maxOutputTokens.trim() ? Number(modelForm.maxOutputTokens) : 0,
    reasoningEffort: modelForm.supportsReasoning && modelForm.reasoningEffort ? modelForm.reasoningEffort : null,
    reasoningBudgetTokens:
      modelForm.supportsReasoning && modelForm.reasoningBudgetTokens.trim()
        ? Number(modelForm.reasoningBudgetTokens)
        : null,
    capabilities: {
      contextWindowTokens: modelForm.contextWindowTokens.trim() ? Number(modelForm.contextWindowTokens) : null,
      supportsTools: modelForm.supportsTools,
      supportsStreaming: modelForm.supportsStreaming,
      supportsImageInput: modelForm.supportsImageInput,
      supportsReasoning: modelForm.supportsReasoning
    }
  });
  providerStore.selectModel(editorState.providerId, payloadId);

  await providerStore.saveRegistry();

  if (!providerStore.error) {
    providerStore.notice = editorState.mode === "edit" ? "模型已更新。" : "模型已新增。";
    beginEditModel(editorState.providerId, payloadId);
  }
}

async function removeCurrentModel() {
  if (!editorState.providerId || !editorState.modelId) {
    return;
  }

  const providerId = editorState.providerId;
  providerStore.removeModel(providerId, editorState.modelId);
  await providerStore.saveRegistry();

  if (providerStore.error) {
    return;
  }

  providerStore.notice = "模型已删除。";
  beginEditProvider(providerId);
}

watch(
  () => providerForm.protocol,
  (protocol, previous) => {
    if (!providerForm.baseUrl || providerForm.baseUrl === defaultBaseUrlFor(previous ?? protocol)) {
      providerForm.baseUrl = defaultBaseUrlFor(protocol);
    }
  }
);

watch(
  [() => providers.value.map((provider) => provider.id).join("|"), () => currentProvider.value?.id ?? null],
  () => {
    if (!providers.value.length) {
      openProviderId.value = null;

      if (!hasInitializedEditor.value) {
        beginCreateProvider();
        hasInitializedEditor.value = true;
      }

      return;
    }

    if (!openProviderId.value || !providers.value.some((provider) => provider.id === openProviderId.value)) {
      openProviderId.value = currentProvider.value?.id ?? providers.value[0].id;
    }

    if (!hasInitializedEditor.value) {
      beginEditProvider(currentProvider.value?.id ?? providers.value[0].id);
      hasInitializedEditor.value = true;
      return;
    }

    if (editorState.providerId && !providers.value.some((provider) => provider.id === editorState.providerId)) {
      beginEditProvider(currentProvider.value?.id ?? providers.value[0].id);
    }
  },
  { immediate: true }
);
</script>

<template>
  <section class="grid h-full min-h-0 min-w-0 gap-4 lg:grid-cols-[minmax(320px,0.78fr)_minmax(0,1.22fr)]">
    <aside class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.6rem] border border-stone-200/70 bg-white/76 px-4 py-4">
      <div class="flex items-start justify-between gap-3 border-b border-stone-200/70 pb-4">
        <div>
          <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">鎻愪緵鍟?/ 妯″瀷</h2>
        </div>
        <Button size="sm" @click="beginCreateProvider()">
          <Plus class="mr-1 h-3.5 w-3.5" />
          鏂板鎻愪緵鍟?        </Button>
      </div>

      <ScrollArea class="mt-4 min-h-0 flex-1" viewport-class="h-full w-full pr-2">
        <div class="space-y-1.5 pb-1">
          <section
            v-for="provider in providers"
            :key="provider.id"
            class="overflow-hidden rounded-[0.5rem] bg-[#efe4d3]"
          >
            <div class="flex items-center gap-2 px-3 py-2">
              <button
                type="button"
                class="min-w-0 flex-1 text-left"
                @click="toggleProvider(provider.id)"
              >
                <div class="flex items-center justify-between gap-3">
                  <div class="min-w-0 flex flex-1 items-center gap-2">
                    <div class="flex min-w-0 items-center gap-2">
                      <ChevronDown
                        class="h-3.5 w-3.5 shrink-0 text-stone-600 transition-transform duration-200"
                        :class="openProviderId === provider.id ? 'rotate-180' : ''"
                      />
                      <span class="truncate text-sm font-medium text-stone-950">
                        {{ provider.name || "鏈懡鍚嶆彁渚涘晢" }}
                      </span>
                    </div>
                    <span
                      class="shrink-0 rounded-full bg-white/45 px-2 py-0.5 text-[10px] uppercase tracking-[0.14em] text-stone-600"
                    >
                      {{ provider.protocol }}
                    </span>
                    <span class="truncate text-[11px] text-stone-600">
                      {{ provider.models.length }} 涓ā鍨?                    </span>
                  </div>
                </div>
              </button>

              <div class="flex shrink-0 items-center gap-0.5">
                <Button size="sm" variant="ghost" class="px-2" @click.stop="beginCreateModel(provider.id)">
                  <Plus class="h-3.5 w-3.5" />
                </Button>
                <Button size="sm" variant="ghost" class="px-2" @click.stop="beginEditProvider(provider.id)">
                  <Pencil class="h-3.5 w-3.5" />
                </Button>
                <label class="space-y-1 text-xs text-stone-500">
                  <span>涓婁笅鏂囩獥鍙?Tokens</span>
                  <Input
                    :model-value="modelForm.contextWindowTokens"
                    type="number"
                    placeholder="渚嬪 128000"
                    @update:model-value="modelForm.contextWindowTokens = $event"
                  />
                </label>

                <label class="space-y-1 text-xs text-stone-500">
                  <span>鎺ㄧ悊寮哄害</span>
                  <select
                    :value="modelForm.reasoningEffort"
                    class="h-11 w-full rounded-[0.5rem] bg-white px-3 text-sm text-stone-900 outline-none transition-colors"
                    @change="modelForm.reasoningEffort = ($event.target as HTMLSelectElement).value as ProviderReasoningEffort | ''"
                  >
                    <option value="">未设置</option>
                    <option value="minimal">minimal</option>
                    <option value="low">low</option>
                    <option value="medium">medium</option>
                    <option value="high">high</option>
                  </select>
                </label>

                <label class="space-y-1 text-xs text-stone-500">
                  <span>鎺ㄧ悊棰勭畻 Tokens</span>
                  <Input
                    :model-value="modelForm.reasoningBudgetTokens"
                    type="number"
                    placeholder="渚嬪 2048"
                    @update:model-value="modelForm.reasoningBudgetTokens = $event"
                  />
                </label>

                <div class="space-y-2 text-xs text-stone-500 xl:col-span-2">
                  <span class="block">鑳藉姏澹版槑</span>
                  <div class="grid gap-2 sm:grid-cols-2">
                    <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
                      <input v-model="modelForm.supportsStreaming" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
                      <span>鏀寔娴佸紡杈撳嚭</span>
                    </label>
                    <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
                      <input v-model="modelForm.supportsTools" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
                      <span>鏀寔宸ュ叿璋冪敤</span>
                    </label>
                    <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
                      <input v-model="modelForm.supportsImageInput" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
                      <span>鏀寔鍥剧墖杈撳叆</span>
                    </label>
                    <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
                      <input v-model="modelForm.supportsReasoning" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
                      <span>鏀寔鎺ㄧ悊鎺у埗</span>
                    </label>
                  </div>
                </div>
              </div>
            </div>

            <div v-if="openProviderId === provider.id" class="bg-white/54 px-2 py-1.5">
              <div class="space-y-1">
                <div
                  v-for="model in provider.models"
                  :key="model.id"
                  class="flex items-center gap-2 rounded-[0.45rem] px-2 py-1.5 transition"
                  :class="
                    editorState.entity === 'model' && editorState.modelId === model.id
                      ? 'bg-[#f2e7d6]'
                      : 'hover:bg-[#f7f0e5]'
                  "
                >
                  <button
                    type="button"
                    class="min-w-0 flex-1 text-left"
                    @click="beginEditModel(provider.id, model.id)"
                  >
                    <div class="flex items-center gap-2">
                      <div class="truncate text-[13px] font-medium text-stone-800">
                        {{ model.name || "未命名模型" }}
                      </div>
                      <div class="truncate text-[11px] text-stone-500">
                        {{ model.model || "未填写模型 ID" }}
                      </div>
                    </div>
                  </button>

                  <div class="flex shrink-0 items-center gap-0.5">
                    <Button size="sm" variant="ghost" class="px-2" @click.stop="beginEditModel(provider.id, model.id)">
                      <Pencil class="h-3.5 w-3.5" />
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </section>

          <div
            v-if="!providers.length"
            class="rounded-[0.45rem] bg-[#f6efe3] px-4 py-4 text-sm leading-6 text-stone-500"
          >
            褰撳墠杩樻病鏈夋彁渚涘晢锛屽厛浠庝笂鏂规柊澧炰竴涓€?          </div>
        </div>
      </ScrollArea>
    </aside>

    <section class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.6rem] border border-stone-200/70 bg-white/76 px-4 py-4 sm:px-5">
      <div v-if="loading" class="rounded-[0.45rem] bg-[#f6efe3] px-4 py-4 text-sm text-stone-500">
        姝ｅ湪璇诲彇閰嶇疆...
      </div>

      <template v-else>
        <div class="flex flex-wrap items-start justify-between gap-3 border-b border-stone-200/70 pb-4">
          <div>
            <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">
              {{ editorTitle }}
            </h2>
            <p class="mt-1 text-[12px] leading-5 text-stone-500">
              {{ editorDescription }}
            </p>
          </div>

          <div class="flex flex-wrap gap-2">
            <Button
              v-if="editorState.entity === 'provider' && editorState.mode === 'edit' && providers.length > 1"
              variant="ghost"
              @click="removeCurrentProvider()"
            >
              <Trash2 class="mr-1 h-4 w-4" />
              鍒犻櫎
            </Button>
            <Button
              v-if="editorState.entity === 'model' && editorState.mode === 'edit' && activeProvider"
              variant="ghost"
              @click="removeCurrentModel()"
            >
              <Trash2 class="mr-1 h-4 w-4" />
              鍒犻櫎
            </Button>
            <Button
              v-if="editorState.entity === 'provider'"
              variant="secondary"
              :disabled="saving || !providerForm.name.trim() || !providerForm.baseUrl.trim()"
              @click="saveProviderForm()"
            >
              <Save class="mr-1 h-4 w-4" />
              {{ saving ? "淇濆瓨涓?.." : "淇濆瓨" }}
            </Button>
            <Button
              v-else
              variant="secondary"
              :disabled="saving || !modelForm.name.trim() || !modelForm.model.trim()"
              @click="saveModelForm()"
            >
              <Save class="mr-1 h-4 w-4" />
              {{ saving ? "淇濆瓨涓?.." : "淇濆瓨" }}
            </Button>
          </div>
        </div>

        <ScrollArea class="mt-4 min-h-0 flex-1" viewport-class="h-full w-full pr-2">
          <div class="space-y-4 pb-1">
            <section v-if="editorState.entity === 'provider'" class="rounded-[0.45rem] bg-[#f6efe5] px-4 py-4">
  <div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
    <label class="space-y-1 text-xs text-stone-500">
      <span>提供商名称</span>
      <Input
        :model-value="providerForm.name"
        placeholder="例如：DeepSeek"
        @update:model-value="providerForm.name = $event"
      />
    </label>

    <label class="space-y-1 text-xs text-stone-500">
      <span>协议</span>
      <select
        :value="providerForm.protocol"
        class="h-11 w-full rounded-[0.5rem] bg-white px-3 text-sm text-stone-900 outline-none transition-colors"
        @change="providerForm.protocol = ($event.target as HTMLSelectElement).value as ProviderProtocol"
      >
        <option value="openai">openai</option>
        <option value="anthropic">anthropic</option>
      </select>
    </label>

    <label class="space-y-1 text-xs text-stone-500 xl:col-span-2">
      <span>Base URL</span>
      <Input
        :model-value="providerForm.baseUrl"
        placeholder="例如：https://api.openai.com/v1"
        @update:model-value="providerForm.baseUrl = $event"
      />
    </label>
  </div>

  <div class="mt-4 rounded-[0.45rem] bg-white/72 px-4 py-4">
    <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
      API Key
      <Shield class="h-3.5 w-3.5 text-stone-500" />
      <InfoTip text="保存后会写入本地 providers.json，运行时直接从这里读取。" />
    </div>

    <div class="mt-3">
      <label class="space-y-1 text-xs text-stone-500">
        <span>当前密钥</span>
        <Input
          :model-value="providerForm.apiKeyValue"
          type="password"
          placeholder="输入后保存即可"
          @update:model-value="providerForm.apiKeyValue = $event"
        />
      </label>
    </div>
  </div>

  <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
    <p class="text-[12px] leading-5 text-stone-500">
      提供商保存后，可以继续在左侧为它新增或编辑模型。
    </p>
  </div>
</section>

<section v-else class="rounded-[0.45rem] bg-[#f6efe5] px-4 py-4">
  <div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
    <label class="space-y-1 text-xs text-stone-500">
      <span>所属提供商</span>
      <Input :model-value="modelParentProvider?.name ?? ''" disabled />
    </label>

    <label class="space-y-1 text-xs text-stone-500">
      <span>名称</span>
      <Input
        :model-value="modelForm.name"
        placeholder="例如：DeepSeek Chat"
        @update:model-value="modelForm.name = $event"
      />
    </label>

    <label class="space-y-1 text-xs text-stone-500 xl:col-span-2">
      <span>模型 ID</span>
      <Input
        :model-value="modelForm.model"
        placeholder="例如：deepseek-chat"
        @update:model-value="modelForm.model = $event"
      />
    </label>
  </div>

  <button
    type="button"
    class="mt-4 inline-flex items-center gap-1 text-[11px] text-stone-500"
    @click="modelForm.showAdvanced = !modelForm.showAdvanced"
  >
    <ChevronDown class="h-3.5 w-3.5 transition-transform duration-200" :class="modelForm.showAdvanced ? 'rotate-180' : ''" />
    {{ modelForm.showAdvanced ? "收起可选参数" : "展开可选参数" }}
  </button>

  <div v-if="modelForm.showAdvanced" class="mt-3 grid gap-3 xl:grid-cols-2">
    <label class="space-y-1 text-xs text-stone-500">
      <span>Temperature</span>
      <Input
        :model-value="modelForm.temperature"
        type="number"
        placeholder="留空则使用默认值"
        @update:model-value="modelForm.temperature = $event"
      />
    </label>

    <label class="space-y-1 text-xs text-stone-500">
      <span>Max Output Tokens</span>
      <Input
        :model-value="modelForm.maxOutputTokens"
        type="number"
        placeholder="留空则使用通用值 8192"
        @update:model-value="modelForm.maxOutputTokens = $event"
      />
    </label>

    <label class="space-y-1 text-xs text-stone-500">
      <span>上下文窗口 Tokens</span>
      <Input
        :model-value="modelForm.contextWindowTokens"
        type="number"
        placeholder="例如 128000"
        @update:model-value="modelForm.contextWindowTokens = $event"
      />
    </label>

    <label class="space-y-1 text-xs text-stone-500">
      <span>推理强度</span>
      <select
        :value="modelForm.reasoningEffort"
        class="h-11 w-full rounded-[0.5rem] bg-white px-3 text-sm text-stone-900 outline-none transition-colors"
        @change="modelForm.reasoningEffort = ($event.target as HTMLSelectElement).value as ProviderReasoningEffort | ''"
      >
        <option value="">未设置</option>
        <option value="minimal">minimal</option>
        <option value="low">low</option>
        <option value="medium">medium</option>
        <option value="high">high</option>
      </select>
    </label>

    <label class="space-y-1 text-xs text-stone-500">
      <span>推理预算 Tokens</span>
      <Input
        :model-value="modelForm.reasoningBudgetTokens"
        type="number"
        placeholder="例如 2048"
        @update:model-value="modelForm.reasoningBudgetTokens = $event"
      />
    </label>

    <div class="space-y-2 text-xs text-stone-500 xl:col-span-2">
      <span class="block">能力声明</span>
      <div class="grid gap-2 sm:grid-cols-2">
        <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
          <input v-model="modelForm.supportsStreaming" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
          <span>支持流式输出</span>
        </label>
        <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
          <input v-model="modelForm.supportsTools" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
          <span>支持工具调用</span>
        </label>
        <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
          <input v-model="modelForm.supportsImageInput" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
          <span>支持图片输入</span>
        </label>
        <label class="flex items-center gap-2 rounded-[0.45rem] bg-white px-3 py-2 text-sm text-stone-700">
          <input v-model="modelForm.supportsReasoning" type="checkbox" class="h-3.5 w-3.5 accent-stone-700" />
          <span>支持推理控制</span>
        </label>
      </div>
    </div>
  </div>

  <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
    <p class="text-[12px] leading-5 text-stone-500">
      右侧只维护当前模型表单，不再重复展示左侧列表内容。
    </p>
  </div>
</section>

<div v-if="notice" class="rounded-[0.45rem] bg-amber-100/70 px-4 py-3 text-sm text-amber-950">
              {{ notice }}
            </div>
            <div v-if="error" class="rounded-[0.45rem] bg-rose-50 px-4 py-3 text-sm text-rose-800">
              {{ error }}
            </div>
          </div>
        </ScrollArea>
      </template>
    </section>
  </section>
</template>


