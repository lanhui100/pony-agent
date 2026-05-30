<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  Check,
  ChevronDown,
  Image as ImageIcon,
  Pencil,
  Plus,
  Save,
  ScanSearch,
  Shield,
  Trash2,
  Wrench
} from "lucide-vue-next";
import InfoTip from "@/components/InfoTip.vue";
import Button from "@/components/ui/Button.vue";
import ConfirmPopover from "@/components/ui/ConfirmPopover.vue";
import Input from "@/components/ui/Input.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import {
  buildProviderModelConfig,
  resolveCapabilityDeclaration,
  resolveModelUserPolicy,
  useProviderStore
} from "@/stores/providers";
import type {
  ProviderCapabilityPresetId,
  ProviderConfig,
  ProviderModelCapabilityDeclaration,
  ProviderModelCapabilities,
  ProviderModelConfig,
  ProviderProtocol,
  ProviderReasoningEffort,
  ProviderModelUserPolicy
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
  capabilityPreset: ProviderCapabilityPresetId;
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
  mode: "create" | "view" | "edit";
  providerId: string | null;
  modelId: string | null;
};

type ModelCapabilityKey =
  | "supportsStreaming"
  | "supportsTools"
  | "supportsImageInput"
  | "supportsReasoning";

const providerStore = useProviderStore();
const { currentProvider, error, loading, notice, providers, saving } = storeToRefs(providerStore);

const openProviderId = ref<string | null>(null);
const hasInitializedEditor = ref(false);
const modelSaveSucceeded = ref(false);
let modelSaveSuccessTimer: ReturnType<typeof setTimeout> | null = null;

const editorState = reactive<EditorState>({
  entity: "provider",
  mode: "view",
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
  capabilityPreset: "auto",
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

const isProviderEntity = computed(() => editorState.entity === "provider");
const isModelEntity = computed(() => editorState.entity === "model");
const isCreateMode = computed(() => editorState.mode === "create");
const isEditMode = computed(() => editorState.mode === "edit");
const isViewMode = computed(() => editorState.mode === "view");
const isEditing = computed(() => editorState.mode === "create" || editorState.mode === "edit");

const detailProvider = computed(() => {
  if (editorState.providerId) {
    return providers.value.find((provider) => provider.id === editorState.providerId) ?? null;
  }

  return currentProvider.value;
});

const detailModel = computed(() => {
  if (editorState.entity === "model" && editorState.providerId && editorState.modelId) {
    const provider = providers.value.find((item) => item.id === editorState.providerId);
    return provider?.models.find((item) => item.id === editorState.modelId) ?? null;
  }

  const provider = detailProvider.value;
  if (!provider) {
    return null;
  }

  return provider.models.find((model) => model.id === provider.selectedModelId) ?? provider.models[0] ?? null;
});

const editorTitle = computed(() => {
  if (isProviderEntity.value) {
    if (isCreateMode.value) {
      return "新增提供商";
    }

    return isEditMode.value ? "编辑提供商" : "提供商详情";
  }

  if (isCreateMode.value) {
    return "新增模型";
  }

  return isEditMode.value ? "编辑模型" : "模型详情";
});

const editorDescription = computed(() => {
  if (isProviderEntity.value) {
    if (isCreateMode.value) {
      return "先创建提供商，再继续补充一个或多个模型。";
    }

    return isEditMode.value ? "修改提供商连接信息与密钥设置。" : "先查看当前提供商详情，需要时再进入编辑。";
  }

  if (isCreateMode.value) {
    return "模型会挂载到当前选中的提供商下。";
  }

  return isEditMode.value ? "调整模型事实与调用策略。" : "先查看模型能力和策略，再按需编辑。";
});

const capabilityPresetOptions = computed(() => {
  const protocol = detailProvider.value?.protocol ?? "openai";

  if (protocol === "anthropic") {
    return [
      { value: "auto", label: "自动推断" },
      { value: "anthropic-thinking", label: "Claude Thinking" },
      { value: "custom", label: "自定义能力" }
    ] satisfies Array<{ value: ProviderCapabilityPresetId; label: string }>;
  }

  return [
    { value: "auto", label: "自动推断" },
    { value: "open-ai-chat", label: "OpenAI Chat" },
    { value: "open-ai-reasoning", label: "OpenAI Reasoning" },
    { value: "deepseek-chat", label: "DeepSeek Chat" },
    { value: "deepseek-reasoner", label: "DeepSeek Reasoner" },
    { value: "custom", label: "自定义能力" }
  ] satisfies Array<{ value: ProviderCapabilityPresetId; label: string }>;
});

const usesCapabilityPreset = computed(() => modelForm.capabilityPreset !== "custom");

const capabilityOptions = [
  {
    key: "supportsTools",
    label: "工具调用",
    hint: "支持模型发起工具调用",
    icon: Wrench
  },
  {
    key: "supportsImageInput",
    label: "图片输入",
    hint: "支持多模态图片输入",
    icon: ImageIcon
  },
  {
    key: "supportsReasoning",
    label: "推理控制",
    hint: "支持推理强度与预算参数",
    icon: ScanSearch
  }
] as const;

const canDeleteProvider = computed(
  () => isProviderEntity.value && !isCreateMode.value && providers.value.length > 1 && Boolean(detailProvider.value)
);
const canDeleteModel = computed(
  () => isModelEntity.value && !isCreateMode.value && Boolean(detailProvider.value && detailModel.value)
);
const canCreateModel = computed(() => isProviderEntity.value && !isEditing.value && Boolean(detailProvider.value));
const canEditCurrent = computed(() => isViewMode.value && Boolean(detailProvider.value));
const canSaveProvider = computed(
  () => isProviderEntity.value && isEditing.value && !saving.value && Boolean(providerForm.name.trim() && providerForm.baseUrl.trim())
);
const canSaveModel = computed(
  () => isModelEntity.value && isEditing.value && !saving.value && Boolean(modelForm.name.trim() && modelForm.model.trim())
);

const resolvedCapabilityDeclaration = computed(() => resolveFormCapabilityDeclaration());
const resolvedModelCapabilities = computed(() => resolvedCapabilityDeclaration.value.capabilities);
const displayedContextWindowTokens = computed(() =>
  usesCapabilityPreset.value
    ? toPositiveIntegerString(resolvedModelCapabilities.value.contextWindowTokens)
    : modelForm.contextWindowTokens
);

function getModelProtocol() {
  return detailProvider.value?.protocol ?? "openai";
}

function getSafeCapabilities(model: Pick<ProviderModelConfig, "capabilities">) {
  return {
    contextWindowTokens: model.capabilities?.contextWindowTokens ?? null,
    supportsTools: model.capabilities?.supportsTools ?? true,
    supportsStreaming: model.capabilities?.supportsStreaming ?? true,
    supportsImageInput: model.capabilities?.supportsImageInput ?? false,
    supportsReasoning: model.capabilities?.supportsReasoning ?? false
  };
}

function createId(prefix: string) {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}-${crypto.randomUUID()}`;
  }

  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
}

function clearModelSaveSuccess() {
  modelSaveSucceeded.value = false;
  if (modelSaveSuccessTimer) {
    clearTimeout(modelSaveSuccessTimer);
    modelSaveSuccessTimer = null;
  }
}

function setModelSaveSuccess() {
  clearModelSaveSuccess();
  modelSaveSucceeded.value = true;
  modelSaveSuccessTimer = setTimeout(() => {
    modelSaveSucceeded.value = false;
    modelSaveSuccessTimer = null;
  }, 5000);
}

function resetModelActionStates() {
  clearModelSaveSuccess();
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
  modelForm.capabilityPreset = "auto";
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

function toPositiveIntegerString(value: number | null | undefined) {
  return value && value > 0 ? String(value) : "";
}

function parseOptionalPositiveInteger(value: string) {
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) && parsed > 0 ? Math.trunc(parsed) : null;
}

function assignCapabilitiesToForm(capabilities: ProviderModelCapabilities) {
  modelForm.contextWindowTokens = toPositiveIntegerString(capabilities.contextWindowTokens);
  modelForm.supportsTools = capabilities.supportsTools;
  modelForm.supportsStreaming = capabilities.supportsStreaming;
  modelForm.supportsImageInput = capabilities.supportsImageInput;
  modelForm.supportsReasoning = capabilities.supportsReasoning;
}

function resolveFormCapabilityDeclaration(preset = modelForm.capabilityPreset): ProviderModelCapabilityDeclaration {
  return resolveCapabilityDeclaration(getModelProtocol(), modelForm.model.trim(), preset, {
    contextWindowTokens: parseOptionalPositiveInteger(modelForm.contextWindowTokens),
    supportsTools: modelForm.supportsTools,
    supportsStreaming: modelForm.supportsStreaming,
    supportsImageInput: modelForm.supportsImageInput,
    supportsReasoning: modelForm.supportsReasoning
  });
}

function resolveFormUserPolicy(capabilities: ProviderModelCapabilities): ProviderModelUserPolicy {
  return resolveModelUserPolicy(
    {
      temperature: modelForm.temperature.trim() ? Number(modelForm.temperature) : 0,
      maxOutputTokens: modelForm.maxOutputTokens.trim() ? Number(modelForm.maxOutputTokens) : 0,
      reasoningEffort: modelForm.reasoningEffort || null,
      reasoningBudgetTokens: parseOptionalPositiveInteger(modelForm.reasoningBudgetTokens)
    },
    capabilities
  );
}

function fillModelForm(model: ProviderModelConfig) {
  const capabilities = getSafeCapabilities(model);
  const userPolicy = resolveModelUserPolicy(model, capabilities);
  modelForm.id = model.id;
  modelForm.name = model.name;
  modelForm.model = model.model;
  modelForm.capabilityPreset = model.capabilityPreset;
  modelForm.temperature = userPolicy.temperature > 0 ? String(userPolicy.temperature) : "";
  modelForm.maxOutputTokens = userPolicy.maxOutputTokens > 0 ? String(userPolicy.maxOutputTokens) : "";
  modelForm.contextWindowTokens = toPositiveIntegerString(capabilities.contextWindowTokens);
  modelForm.reasoningEffort = capabilities.supportsReasoning ? (userPolicy.reasoningEffort ?? "") : "";
  modelForm.reasoningBudgetTokens = capabilities.supportsReasoning
    ? toPositiveIntegerString(userPolicy.reasoningBudgetTokens)
    : "";
  modelForm.supportsTools = capabilities.supportsTools;
  modelForm.supportsStreaming = capabilities.supportsStreaming;
  modelForm.supportsImageInput = capabilities.supportsImageInput;
  modelForm.supportsReasoning = capabilities.supportsReasoning;
  modelForm.showAdvanced =
    model.temperature > 0 ||
    model.maxOutputTokens > 0 ||
    !!capabilities.contextWindowTokens ||
    !!model.reasoningEffort ||
    !!model.reasoningBudgetTokens ||
    !capabilities.supportsStreaming ||
    !capabilities.supportsTools ||
    capabilities.supportsImageInput ||
    capabilities.supportsReasoning;
}

function toggleProvider(providerId: string) {
  openProviderId.value = openProviderId.value === providerId ? null : providerId;
}

function beginViewProvider(providerId: string) {
  const provider = findProvider(providerId);
  if (!provider) {
    return;
  }

  resetModelActionStates();
  providerStore.selectProvider(providerId);
  openProviderId.value = providerId;
  editorState.entity = "provider";
  editorState.mode = "view";
  editorState.providerId = providerId;
  editorState.modelId = null;
}

function beginViewModel(providerId: string, modelId: string) {
  const provider = findProvider(providerId);
  const model = findModel(providerId, modelId);
  if (!provider || !model) {
    return;
  }

  resetModelActionStates();
  providerStore.selectModel(providerId, modelId);
  openProviderId.value = providerId;
  editorState.entity = "model";
  editorState.mode = "view";
  editorState.providerId = providerId;
  editorState.modelId = modelId;
}

function beginCreateProvider() {
  resetModelActionStates();
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

  resetModelActionStates();
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

  resetModelActionStates();
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

  resetModelActionStates();
  providerStore.selectModel(providerId, modelId);
  openProviderId.value = providerId;
  fillModelForm(model);
  editorState.entity = "model";
  editorState.mode = "edit";
  editorState.providerId = providerId;
  editorState.modelId = modelId;
}

function beginEditCurrent() {
  if (!detailProvider.value) {
    return;
  }

  if (isProviderEntity.value) {
    beginEditProvider(detailProvider.value.id);
    return;
  }

  if (detailModel.value) {
    beginEditModel(detailProvider.value.id, detailModel.value.id);
  }
}

function cancelEditing() {
  if (isProviderEntity.value) {
    if (detailProvider.value) {
      beginViewProvider(detailProvider.value.id);
      return;
    }

    beginCreateProvider();
    return;
  }

  if (editorState.providerId && detailModel.value) {
    beginViewModel(editorState.providerId, detailModel.value.id);
    return;
  }

  if (detailProvider.value) {
    beginViewProvider(detailProvider.value.id);
    return;
  }

  beginCreateProvider();
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
    beginViewProvider(providerId);
  }
}

async function removeCurrentProvider() {
  if (!detailProvider.value) {
    return;
  }

  const removingId = detailProvider.value.id;
  providerStore.removeProvider(removingId);
  await providerStore.saveRegistry();

  if (providerStore.error) {
    return;
  }

  providerStore.notice = "提供商已删除。";
  const nextProvider = providers.value[0] ?? null;

  if (nextProvider) {
    beginViewProvider(nextProvider.id);
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
  const declaration = resolvedCapabilityDeclaration.value;
  const userPolicy = resolveFormUserPolicy(declaration.capabilities);

  providerStore.upsertModel(
    editorState.providerId,
    buildProviderModelConfig(
      {
        id: payloadId,
        name,
        model: modelIdValue
      },
      declaration,
      userPolicy
    )
  );
  providerStore.selectModel(editorState.providerId, payloadId);

  await providerStore.saveRegistry();

  if (!providerStore.error) {
    providerStore.notice = editorState.mode === "edit" ? "模型已更新。" : "模型已新增。";
    beginViewModel(editorState.providerId, payloadId);
    setModelSaveSuccess();
  }
}

async function removeCurrentModel() {
  if (!editorState.providerId || !detailModel.value) {
    return;
  }

  const providerId = editorState.providerId;
  providerStore.removeModel(providerId, detailModel.value.id);
  await providerStore.saveRegistry();

  if (providerStore.error) {
    return;
  }

  providerStore.notice = "模型已删除。";
  beginViewProvider(providerId);
}

function enabledCapabilityOptions(model: ProviderModelConfig) {
  const capabilities = getSafeCapabilities(model);
  return capabilityOptions.filter((option) => capabilities[option.key]);
}

function toggleCapability(key: ModelCapabilityKey) {
  if (usesCapabilityPreset.value) {
    return;
  }

  modelForm[key] = !modelForm[key];
}

function presetLabel(preset: ProviderCapabilityPresetId) {
  const labels: Record<ProviderCapabilityPresetId, string> = {
    auto: "自动推断",
    "open-ai-chat": "OpenAI Chat",
    "open-ai-reasoning": "OpenAI Reasoning",
    "anthropic-thinking": "Claude Thinking",
    "deepseek-chat": "DeepSeek Chat",
    "deepseek-reasoner": "DeepSeek Reasoner",
    custom: "自定义能力"
  };

  return labels[preset];
}

function boolLabel(value: boolean) {
  return value ? "开启" : "关闭";
}

function numberLabel(value: number | null | undefined, empty = "默认") {
  return value && value > 0 ? String(value) : empty;
}

function reasoningLabel(value: ProviderReasoningEffort | null | undefined, supportsReasoning: boolean) {
  if (!supportsReasoning) {
    return "未启用";
  }

  return value ?? "跟随模型默认";
}

function providerApiKeySummary(provider: ProviderConfig) {
  if (provider.apiKeyPresent) {
    return "已保存到应用密钥存储";
  }

  if (provider.apiKeyValue.trim()) {
    return "当前页面已填写，保存后写入密钥存储";
  }

  return "未设置";
}

onBeforeUnmount(() => {
  clearModelSaveSuccess();
});

watch(
  () => providerForm.protocol,
  (protocol, previous) => {
    if (!providerForm.baseUrl || providerForm.baseUrl === defaultBaseUrlFor(previous ?? protocol)) {
      providerForm.baseUrl = defaultBaseUrlFor(protocol);
    }
  }
);

watch(
  () => resolvedModelCapabilities.value.supportsReasoning,
  (supportsReasoning) => {
    if (!supportsReasoning) {
      modelForm.reasoningEffort = "";
      modelForm.reasoningBudgetTokens = "";
    }
  }
);

watch(
  () => modelForm.capabilityPreset,
  (preset, previous) => {
    if (preset === "custom") {
      const seedPreset = previous && previous !== "custom" ? previous : "custom";
      assignCapabilitiesToForm(resolveFormCapabilityDeclaration(seedPreset).capabilities);
      return;
    }

    if (previous === "custom") {
      assignCapabilitiesToForm(resolveFormCapabilityDeclaration(preset).capabilities);
    }

    modelForm.contextWindowTokens = "";
  }
);

watch(
  () => modelForm.model,
  () => {
    if (!usesCapabilityPreset.value) {
      return;
    }

    modelForm.contextWindowTokens = "";
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
      beginViewProvider(currentProvider.value?.id ?? providers.value[0].id);
      hasInitializedEditor.value = true;
      return;
    }

    if (editorState.providerId && !findProvider(editorState.providerId)) {
      beginViewProvider(currentProvider.value?.id ?? providers.value[0].id);
      return;
    }

    if (
      editorState.entity === "model" &&
      editorState.mode !== "create" &&
      editorState.providerId &&
      editorState.modelId &&
      !findModel(editorState.providerId, editorState.modelId)
    ) {
      beginViewProvider(editorState.providerId);
    }
  },
  { immediate: true }
);
</script>

<template>
  <section class="grid h-full min-h-0 min-w-0 gap-3 lg:grid-cols-[minmax(280px,0.68fr)_minmax(0,1.32fr)]">
    <aside
      class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.55rem] bg-[#f1e2cf]/88 px-3 py-3.5"
    >
      <div class="flex items-start justify-between gap-2.5 pb-3">
        <div>
          <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">提供商 / 模型</h2>
          <p class="mt-1 text-[12px] leading-5 text-stone-500">左侧只负责切换对象，右侧查看或编辑详情。</p>
        </div>
        <Button size="sm" variant="ghost" class="shrink-0" @click="beginCreateProvider()">
          <Plus class="mr-1 h-3.5 w-3.5" />
          新增提供商
        </Button>
      </div>

      <ScrollArea class="mt-3 min-h-0 flex-1" viewport-class="h-full w-full pr-1">
        <div class="space-y-1.5">
          <section
            v-for="provider in providers"
            :key="provider.id"
            class="overflow-hidden rounded-[0.45rem] bg-white/30"
          >
            <button
              type="button"
              class="flex w-full items-start justify-between gap-2.5 rounded-[0.35rem] px-2.5 py-2 text-left transition hover:bg-white/74"
              @click="
                if (openProviderId !== provider.id) {
                  openProviderId = provider.id;
                } else {
                  toggleProvider(provider.id);
                }
                beginViewProvider(provider.id);
              "
            >
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <ChevronDown
                    class="h-4 w-4 shrink-0 text-stone-400 transition-transform duration-200"
                    :class="openProviderId === provider.id ? 'rotate-180' : ''"
                  />
                  <span class="truncate text-sm font-medium text-stone-950">
                    {{ provider.name || "未命名提供商" }}
                  </span>
                </div>
                <div class="mt-0.5 flex flex-wrap items-center gap-1.5 pl-6 text-[10px] text-stone-500">
                  <span class="uppercase tracking-[0.14em]">{{ provider.protocol }}</span>
                  <span>{{ provider.models.length }} 个模型</span>
                </div>
              </div>
            </button>

            <div v-if="openProviderId === provider.id" class="px-1.5 pb-1.5">
              <button
                type="button"
                class="flex w-full rounded-[0.35rem] px-2 py-1.5 text-left transition hover:bg-white/74"
                :class="
                  editorState.entity === 'provider' && editorState.providerId === provider.id
                    ? 'bg-white/78 text-stone-950'
                    : 'bg-transparent'
                "
                @click="beginViewProvider(provider.id)"
              >
                <div class="min-w-0">
                  <div class="text-[12px] font-medium text-stone-800">提供商详情</div>
                  <div class="mt-0.5 truncate text-[11px] text-stone-500">{{ provider.baseUrl }}</div>
                </div>
              </button>

              <div class="mt-0.5 space-y-0.5">
                <button
                  v-for="model in provider.models"
                  :key="model.id"
                  type="button"
                  class="flex w-full items-start rounded-[0.35rem] px-2 py-1.5 text-left transition hover:bg-white/74"
                  :class="
                    editorState.entity === 'model' && editorState.modelId === model.id
                      ? 'bg-white/78 text-stone-950'
                      : 'bg-transparent'
                  "
                  @click="beginViewModel(provider.id, model.id)"
                >
                  <div class="min-w-0">
                    <div class="truncate text-[12px] font-medium text-stone-800">
                      {{ model.name || "未命名模型" }}
                    </div>
                    <div class="mt-0.5 truncate text-[11px] text-stone-500">
                      {{ model.model || "未填写模型 ID" }}
                    </div>
                  </div>
                </button>
              </div>
            </div>
          </section>

          <div
            v-if="!providers.length"
            class="rounded-[0.45rem] bg-white/58 px-3.5 py-3 text-sm leading-6 text-stone-500"
          >
            当前还没有提供商，先从上方新增一个。
          </div>
        </div>
      </ScrollArea>
    </aside>

    <section
      class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.55rem] px-3 py-3.5 sm:px-4"
    >
      <div v-if="loading" class="rounded-[0.45rem] bg-white/70 px-3.5 py-3 text-sm text-stone-500">
        正在读取配置...
      </div>

      <template v-else>
        <div class="flex flex-wrap items-start justify-between gap-2.5 pb-3">
          <div>
            <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">
              {{ editorTitle }}
            </h2>
            <p class="mt-1 text-[12px] leading-5 text-stone-500">
              {{ editorDescription }}
            </p>
          </div>

          <div class="flex flex-wrap gap-1.5">
            <Button v-if="canCreateModel && detailProvider" size="sm" variant="ghost" @click="beginCreateModel(detailProvider.id)">
              <Plus class="mr-1 h-4 w-4" />
              新增模型
            </Button>
            <Button v-if="canEditCurrent" size="sm" variant="ghost" @click="beginEditCurrent()">
              <Pencil class="mr-1 h-4 w-4" />
              编辑
            </Button>
            <Button v-if="isEditing" size="sm" variant="ghost" @click="cancelEditing()">
              取消
            </Button>
            <ConfirmPopover
              v-if="canDeleteProvider"
              :title="`删除提供商「${detailProvider?.name || detailProvider?.id || '当前提供商'}」？`"
              description="此操作不可撤销。"
              side="bottom"
              align="end"
              @confirm="removeCurrentProvider()"
            >
              <Button size="sm" variant="ghost">
                <Trash2 class="mr-1 h-4 w-4" />
                删除
              </Button>
            </ConfirmPopover>
            <ConfirmPopover
              v-if="canDeleteModel"
              :title="`删除模型「${detailModel?.name || detailModel?.model || '当前模型'}」？`"
              description="此操作不可撤销。"
              side="bottom"
              align="end"
              @confirm="removeCurrentModel()"
            >
              <Button size="sm" variant="ghost">
                <Trash2 class="mr-1 h-4 w-4" />
                删除
              </Button>
            </ConfirmPopover>
            <Button v-if="canSaveProvider" size="sm" variant="secondary" @click="saveProviderForm()">
              <Save class="mr-1 h-4 w-4" />
              {{ saving ? "保存中..." : "保存" }}
            </Button>
            <Button v-if="canSaveModel" size="sm" variant="secondary" @click="saveModelForm()">
              <Check v-if="modelSaveSucceeded && !saving" class="mr-1 h-4 w-4" />
              <Save v-else class="mr-1 h-4 w-4" />
              {{ saving ? "保存中..." : modelSaveSucceeded ? "已保存" : "保存" }}
            </Button>
          </div>
        </div>

        <ScrollArea class="mt-3 min-h-0 flex-1" viewport-class="h-full w-full pr-1">
          <div class="space-y-3 pb-1">
            <div v-if="isProviderEntity" class="config-form space-y-3">
              <template v-if="isEditing">
                <div class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                  <label class="space-y-1 text-[11px] text-stone-500">
                    <span>提供商名称</span>
                    <Input
                      :model-value="providerForm.name"
                      placeholder="例如：DeepSeek"
                      @update:model-value="providerForm.name = $event"
                    />
                  </label>

                  <label class="space-y-1 text-[11px] text-stone-500">
                    <span>协议</span>
                    <select
                      :value="providerForm.protocol"
                      class="config-select"
                      @change="providerForm.protocol = ($event.target as HTMLSelectElement).value as ProviderProtocol"
                    >
                      <option value="openai">openai</option>
                      <option value="anthropic">anthropic</option>
                    </select>
                  </label>

                  <label class="space-y-1 text-[11px] text-stone-500 xl:col-span-2">
                    <span>Base URL</span>
                    <Input
                      :model-value="providerForm.baseUrl"
                      placeholder="例如：https://api.openai.com/v1"
                      @update:model-value="providerForm.baseUrl = $event"
                    />
                  </label>
                </div>

                <div class="mt-3 rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="flex items-center gap-1.5 text-[13px] font-medium text-stone-900">
                    API Key
                    <Shield class="h-3.5 w-3.5 text-stone-500" />
                    <InfoTip
                      text="输入新密钥后会保存到应用密钥存储；providers.json 只保留非敏感配置，运行时会优先读取密钥存储，必要时才回退到环境变量。"
                    />
                  </div>

                  <div class="mt-2.5">
                    <label class="space-y-1 text-[11px] text-stone-500">
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
              </template>

              <template v-else-if="detailProvider">
                <div class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                    <div class="text-sm font-medium text-stone-900">基础信息</div>
                    <div class="mt-2.5 space-y-2.5 text-[13px] leading-6 text-stone-600">
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">名称</div>
                        <div class="mt-1 text-stone-900">{{ detailProvider.name || "未命名提供商" }}</div>
                      </div>
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">协议</div>
                        <div class="mt-1 uppercase text-stone-900">{{ detailProvider.protocol }}</div>
                      </div>
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Base URL</div>
                        <div class="mt-1 break-words text-stone-900">{{ detailProvider.baseUrl }}</div>
                      </div>
                    </div>
                  </section>

                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                    <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                      密钥与环境
                      <Shield class="h-3.5 w-3.5 text-stone-500" />
                    </div>
                    <div class="mt-2.5 space-y-2.5 text-[13px] leading-6 text-stone-600">
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">环境变量名</div>
                        <div class="mt-1 text-stone-900">{{ detailProvider.apiKeyEnvVar }}</div>
                      </div>
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">密钥状态</div>
                        <div class="mt-1 text-stone-900">{{ providerApiKeySummary(detailProvider) }}</div>
                      </div>
                    </div>
                  </section>
                </div>

                <section class="mt-3 rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="flex items-center justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium text-stone-900">模型列表</div>
                      <div class="mt-0.5 text-[11px] text-stone-500">
                        当前挂载 {{ detailProvider.models.length }} 个模型，可从右上角新增模型。
                      </div>
                    </div>
                  </div>

                  <div class="mt-2.5 grid gap-2.5 lg:grid-cols-2">
                    <div
                      v-for="model in detailProvider.models"
                      :key="model.id"
                      class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5"
                    >
                      <div class="truncate text-[13px] font-medium text-stone-900">{{ model.name || "未命名模型" }}</div>
                      <div class="mt-1 truncate text-[12px] text-stone-500">{{ model.model || "未填写模型 ID" }}</div>
                      <div class="mt-1.5 flex flex-wrap gap-1">
                        <span
                          v-for="option in enabledCapabilityOptions(model)"
                          :key="option.key"
                          class="inline-flex items-center gap-1 rounded-full bg-white/78 px-1.5 py-0.5 text-[10px] text-stone-600"
                        >
                          <component :is="option.icon" class="h-3 w-3" />
                          {{ option.label }}
                        </span>
                      </div>
                    </div>
                  </div>
                </section>
              </template>
            </div>

            <div v-else class="config-form space-y-3">
              <template v-if="isEditing">
                <div class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                  <label class="space-y-1 text-[11px] text-stone-500">
                    <span>所属提供商</span>
                    <Input :model-value="detailProvider?.name ?? ''" disabled />
                  </label>

                  <label class="space-y-1 text-[11px] text-stone-500">
                    <span>名称</span>
                    <Input
                      :model-value="modelForm.name"
                      placeholder="例如：DeepSeek Chat"
                      @update:model-value="modelForm.name = $event"
                    />
                  </label>

                  <label class="space-y-1 text-[11px] text-stone-500 xl:col-span-2">
                    <span>模型 ID</span>
                    <Input
                      :model-value="modelForm.model"
                      placeholder="例如：deepseek-chat"
                      @update:model-value="modelForm.model = $event"
                    />
                  </label>

                  <label class="space-y-1 text-[11px] text-stone-500 xl:col-span-2">
                    <span>能力预设</span>
                    <select
                      :value="modelForm.capabilityPreset"
                      class="config-select"
                      @change="modelForm.capabilityPreset = ($event.target as HTMLSelectElement).value as ProviderCapabilityPresetId"
                    >
                      <option v-for="option in capabilityPresetOptions" :key="option.value" :value="option.value">
                        {{ option.label }}
                      </option>
                    </select>
                    <p class="mt-1 text-[10px] leading-4.5 text-stone-500">
                      预设用于声明模型事实；只有切到“自定义能力”时，才手动覆盖上下文窗口和能力开关。
                    </p>
                  </label>
                </div>

                <button
                  type="button"
                  class="mt-3 inline-flex items-center gap-1 text-[11px] text-stone-500"
                  @click="modelForm.showAdvanced = !modelForm.showAdvanced"
                >
                  <ChevronDown class="h-3.5 w-3.5 transition-transform duration-200" :class="modelForm.showAdvanced ? 'rotate-180' : ''" />
                  {{ modelForm.showAdvanced ? "收起可选参数" : "展开可选参数" }}
                </button>

                <div v-if="modelForm.showAdvanced" class="mt-2.5 space-y-3">
                  <div class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                    <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                      模型事实
                      <InfoTip
                        text="图标亮起表示该能力开启。预设模式下这些能力由模型事实驱动；切换到自定义后才允许手动改写。"
                      />
                    </div>
                    <p class="mt-1.5 text-[11px] leading-5 text-stone-500">
                      {{ usesCapabilityPreset ? "当前为预设驱动，能力只读。" : "当前为自定义能力，改动会直接写入 providers.json。" }}
                    </p>
                    <div class="mt-2.5 flex flex-wrap gap-1.5">
                      <button
                        v-for="option in capabilityOptions"
                        :key="option.key"
                        type="button"
                        class="inline-flex h-9 w-9 items-center justify-center rounded-[0.45rem] transition-colors disabled:cursor-not-allowed disabled:opacity-55"
                        :class="
                          resolvedModelCapabilities[option.key]
                            ? 'bg-stone-900 text-stone-50'
                            : 'bg-stone-100 text-stone-500 hover:bg-white hover:text-stone-700'
                        "
                        :title="`${option.label}：${option.hint}`"
                        :aria-label="option.label"
                        :disabled="usesCapabilityPreset"
                        @click="toggleCapability(option.key)"
                      >
                        <component :is="option.icon" class="h-4 w-4" />
                      </button>
                    </div>
                    <label class="mt-3 block space-y-1 text-[11px] text-stone-500">
                      <span>上下文窗口 Tokens</span>
                      <Input
                        :model-value="displayedContextWindowTokens"
                        type="number"
                        :disabled="usesCapabilityPreset"
                        :placeholder="usesCapabilityPreset ? '由能力预设自动决定' : '例如 128000'"
                        @update:model-value="modelForm.contextWindowTokens = $event"
                      />
                    </label>
                  </div>

                  <div class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                    <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                      用户策略
                      <InfoTip
                        text="这里配置调用时的生成策略，不声明模型本身具备什么能力。推理相关参数只有在模型事实允许时才会生效。"
                      />
                    </div>
                    <div class="mt-2.5 grid gap-3 xl:grid-cols-2">
                      <label class="space-y-1 text-[11px] text-stone-500">
                        <span>Temperature</span>
                        <Input
                          :model-value="modelForm.temperature"
                          type="number"
                          placeholder="留空则使用默认值"
                          @update:model-value="modelForm.temperature = $event"
                        />
                      </label>

                      <label class="space-y-1 text-[11px] text-stone-500">
                        <span>Max Output Tokens</span>
                        <Input
                          :model-value="modelForm.maxOutputTokens"
                          type="number"
                          placeholder="留空则使用通用值 8192"
                          @update:model-value="modelForm.maxOutputTokens = $event"
                        />
                      </label>

                      <label v-if="resolvedModelCapabilities.supportsReasoning" class="space-y-1 text-[11px] text-stone-500">
                        <span>推理强度</span>
                        <select
                          :value="modelForm.reasoningEffort"
                          class="config-select"
                          @change="modelForm.reasoningEffort = ($event.target as HTMLSelectElement).value as ProviderReasoningEffort | ''"
                        >
                          <option value="">跟随模型默认值</option>
                          <option value="minimal">minimal</option>
                          <option value="low">low</option>
                          <option value="medium">medium</option>
                          <option value="high">high</option>
                        </select>
                      </label>

                      <label v-if="resolvedModelCapabilities.supportsReasoning" class="space-y-1 text-[11px] text-stone-500">
                        <span>推理预算 Tokens</span>
                        <Input
                          :model-value="modelForm.reasoningBudgetTokens"
                          type="number"
                          placeholder="例如 2048"
                          @update:model-value="modelForm.reasoningBudgetTokens = $event"
                        />
                      </label>
                    </div>
                  </div>
                </div>
              </template>

              <template v-else-if="detailModel">
                <div class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                    <div class="text-sm font-medium text-stone-900">模型信息</div>
                    <div class="mt-2.5 space-y-2.5 text-[13px] leading-6 text-stone-600">
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">所属提供商</div>
                        <div class="mt-1 text-stone-900">{{ detailProvider?.name || "未命名提供商" }}</div>
                      </div>
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">名称</div>
                        <div class="mt-1 text-stone-900">{{ detailModel.name || "未命名模型" }}</div>
                      </div>
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">模型 ID</div>
                        <div class="mt-1 break-words text-stone-900">{{ detailModel.model || "未填写模型 ID" }}</div>
                      </div>
                      <div>
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">能力预设</div>
                        <div class="mt-1 text-stone-900">{{ presetLabel(detailModel.capabilityPreset) }}</div>
                      </div>
                    </div>
                  </section>

                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                    <div class="text-sm font-medium text-stone-900">能力概览</div>
                    <div class="mt-2.5 grid gap-2.5 sm:grid-cols-2">
                      <div
                        v-for="option in capabilityOptions"
                        :key="option.key"
                        class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5"
                      >
                        <div class="flex items-center gap-2 text-stone-900">
                          <component :is="option.icon" class="h-4 w-4" />
                          <span class="text-[13px] font-medium">{{ option.label }}</span>
                        </div>
                        <div class="mt-1.5 text-[12px] text-stone-500">
                          {{ boolLabel(detailModel.capabilities[option.key]) }}
                        </div>
                      </div>
                    </div>
                  </section>
                </div>

                <section class="mt-3 rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="text-sm font-medium text-stone-900">调用策略</div>
                  <div class="mt-2.5 grid gap-2.5 lg:grid-cols-2">
                    <div class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5">
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">上下文窗口</div>
                      <div class="mt-1 text-[13px] text-stone-900">
                        {{ numberLabel(detailModel.capabilities.contextWindowTokens, "自动推断") }}
                      </div>
                    </div>
                    <div class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5">
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Temperature</div>
                      <div class="mt-1 text-[13px] text-stone-900">{{ numberLabel(detailModel.temperature, "默认") }}</div>
                    </div>
                    <div class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5">
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">Max Output Tokens</div>
                      <div class="mt-1 text-[13px] text-stone-900">{{ numberLabel(detailModel.maxOutputTokens, "默认") }}</div>
                    </div>
                    <div class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5">
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">推理强度</div>
                      <div class="mt-1 text-[13px] text-stone-900">
                        {{ reasoningLabel(detailModel.reasoningEffort, detailModel.capabilities.supportsReasoning) }}
                      </div>
                    </div>
                    <div class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5 lg:col-span-2">
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">推理预算 Tokens</div>
                      <div class="mt-1 text-[13px] text-stone-900">
                        {{
                          detailModel.capabilities.supportsReasoning
                            ? numberLabel(detailModel.reasoningBudgetTokens, "跟随模型默认")
                            : "未启用"
                        }}
                      </div>
                    </div>
                  </div>
                </section>
              </template>
            </div>

            <div v-if="notice" class="rounded-[0.45rem] bg-amber-50/85 px-3.5 py-3 text-sm text-amber-950">
              {{ notice }}
            </div>
            <div v-if="error" class="rounded-[0.45rem] bg-rose-50/90 px-3.5 py-3 text-sm text-rose-800">
              {{ error }}
            </div>
          </div>
        </ScrollArea>
      </template>
    </section>
  </section>
</template>

<style scoped>
.config-form :deep(input) {
  height: 2.5rem;
  border-radius: 0.45rem;
  border: none;
  background: rgb(255 255 255 / 0.82);
  padding-inline: 0.75rem;
  box-shadow: none;
}

.config-form :deep(input:focus-visible) {
  background: rgb(255 255 255 / 0.98);
  outline: none;
}

.config-select {
  height: 2.5rem;
  width: 100%;
  border-radius: 0.45rem;
  border: none;
  background: rgb(255 255 255 / 0.82);
  padding-inline: 0.75rem;
  font-size: 0.875rem;
  color: rgb(28 25 23);
  outline: none;
  transition:
    background-color 160ms ease;
}

.config-select:focus {
  outline: none;
  background: rgb(255 255 255 / 0.98);
}
</style>
