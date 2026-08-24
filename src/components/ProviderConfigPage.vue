<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  Brain,
  Check,
  ChevronDown,
  Image as ImageIcon,
  Mic,
  Pencil,
  Plus,
  Save,
  Shield,
  Trash2,
  Type,
  Video,
} from "lucide-vue-next";
import InfoTip from "@/components/InfoTip.vue";
import Button from "@/components/ui/Button.vue";
import ConfirmPopover from "@/components/ui/ConfirmPopover.vue";
import Input from "@/components/ui/Input.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import Tooltip from "@/components/ui/Tooltip.vue";
import {
  buildProviderModelConfig,
  createDefaultCapabilities,
  defaultBaseUrlFor,
  resolveCapabilityDeclaration,
  resolveModelUserPolicy,
  useProviderStore,
} from "@/stores/providers";
import type {
  ProviderAuthType,
  ProviderConfig,
  ProviderModelCapabilities,
  ProviderModelConfig,
  ProviderProtocol,
  ProviderProtocolEndpoint,
} from "@/types/provider";

type ProviderEndpointFormState = {
  enabled: boolean;
  baseUrl: string;
  authType: ProviderAuthType;
};

type ProviderFormState = {
  name: string;
  endpoints: Record<ProviderProtocol, ProviderEndpointFormState>;
  apiKeyValue: string;
};

type ModelFormState = {
  id: string | null;
  name: string;
  model: string;
  contextWindowTokens: string;
  maxOutputTokens: string;
  supportsReasoning: boolean;
  supportsImageInput: boolean;
  supportsVideoInput: boolean;
  supportsAudioInput: boolean;
  supportsTextOutput: boolean;
  supportsImageOutput: boolean;
  supportsVideoOutput: boolean;
  supportsAudioOutput: boolean;
};

type EditorState = {
  entity: "provider" | "model";
  mode: "create" | "view" | "edit";
  providerId: string | null;
  modelId: string | null;
};

type ModelCapabilityToggleKey =
  | "supportsReasoning"
  | "supportsImageInput"
  | "supportsVideoInput"
  | "supportsAudioInput"
  | "supportsTextOutput"
  | "supportsImageOutput"
  | "supportsVideoOutput"
  | "supportsAudioOutput";

const providerStore = useProviderStore();
const { currentProvider, error, loading, notice, providers, saving } =
  storeToRefs(providerStore);

// ADR 0013：右侧双一级折叠区状态（组件内受控，不持久化——配置页 tab 切走即随
// ConfigPage 的 v-if 卸载整体重置）。expandedModelId 为当前展开"模型配置详情"
// 的模型行；modelCreateOpen 为列表顶部的新增模型表单卡。
// 迭代三：两一级区改为手风琴——同时至多展开一个，默认展开提供商详情。
const providerSectionOpen = ref(true);
const modelSectionOpen = ref(false);
const expandedModelId = ref<string | null>(null);
const modelCreateOpen = ref(false);
const hasInitializedEditor = ref(false);
const modelSaveSucceeded = ref(false);
let modelSaveSuccessTimer: ReturnType<typeof setTimeout> | null = null;

const editorState = reactive<EditorState>({
  entity: "provider",
  mode: "view",
  providerId: null,
  modelId: null,
});

const providerForm = reactive<ProviderFormState>({
  name: "",
  endpoints: {
    openai: {
      enabled: true,
      baseUrl: defaultBaseUrlFor("openai"),
      authType: "auto",
    },
    anthropic: {
      enabled: false,
      baseUrl: defaultBaseUrlFor("anthropic"),
      authType: "x-api-key",
    },
  },
  apiKeyValue: "",
});

const modelForm = reactive<ModelFormState>({
  id: null,
  name: "",
  model: "",
  contextWindowTokens: "256000",
  maxOutputTokens: "64000",
  supportsReasoning: false,
  supportsImageInput: false,
  supportsVideoInput: false,
  supportsAudioInput: false,
  supportsTextOutput: true,
  supportsImageOutput: false,
  supportsVideoOutput: false,
  supportsAudioOutput: false,
});

const inputCapabilityOptions = [
  {
    key: "supportsImageInput",
    label: "图片",
    icon: ImageIcon,
  },
  {
    key: "supportsVideoInput",
    label: "视频",
    icon: Video,
  },
  {
    key: "supportsAudioInput",
    label: "语音",
    icon: Mic,
  },
] satisfies Array<{
  key: ModelCapabilityToggleKey;
  label: string;
  icon: typeof Brain;
}>;

const outputCapabilityOptions = [
  {
    key: "supportsTextOutput",
    label: "文字",
    icon: Type,
  },
  {
    key: "supportsImageOutput",
    label: "图片",
    icon: ImageIcon,
  },
  {
    key: "supportsVideoOutput",
    label: "视频",
    icon: Video,
  },
  {
    key: "supportsAudioOutput",
    label: "语音",
    icon: Mic,
  },
  {
    key: "supportsReasoning",
    label: "思考模型",
    icon: Brain,
  },
] satisfies Array<{
  key: ModelCapabilityToggleKey;
  label: string;
  icon: typeof Brain;
}>;

const endpointOrder: ProviderProtocol[] = ["openai", "anthropic"];

// 行尾/头部弱化图标动作钮（ADR 0013 迭代二）：小尺寸、低对比，仅 hover 增强。
const ICON_ACTION_CLASS =
  "inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-[0.35rem] bg-transparent text-stone-400 transition hover:bg-[#f7e3bf] hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70 disabled:cursor-not-allowed disabled:text-stone-300 motion-reduce:transition-none";

// ADR 0013 迭代：模型参数以 K/M 常用单位呈现（去"tokens"字样），并提供
// 徽标式常规项一键填入（十进制口径：256K=256000，与既有默认值一致）。
const CONTEXT_TOKEN_PRESETS = [32000, 64000, 128000, 256000, 512000, 1000000];
const MAX_OUTPUT_TOKEN_PRESETS = [4000, 8000, 16000, 32000, 64000, 128000];

function formatTokenUnit(value: number | null | undefined, fallback: string): string {
  const tokens = value && value > 0 ? value : null;
  if (!tokens) {
    return fallback;
  }
  if (tokens >= 1_000_000) {
    const millions = tokens / 1_000_000;
    return Number.isInteger(millions) ? `${millions}M` : `${millions.toFixed(1)}M`;
  }
  if (tokens >= 1_000) {
    const thousands = tokens / 1_000;
    return Number.isInteger(thousands) ? `${thousands}K` : `${thousands.toFixed(1)}K`;
  }
  return String(tokens);
}

const isProviderEntity = computed(() => editorState.entity === "provider");
const isModelEntity = computed(() => editorState.entity === "model");
const isCreateMode = computed(() => editorState.mode === "create");
const isViewMode = computed(() => editorState.mode === "view");
const isEditing = computed(
  () => editorState.mode === "create" || editorState.mode === "edit",
);

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

const providerEnabledProtocols = computed(() =>
  endpointOrder.filter((protocol) => providerForm.endpoints[protocol].enabled),
);

const canDeleteProvider = computed(
  () =>
    isProviderEntity.value &&
    !isCreateMode.value &&
    providers.value.length > 1 &&
    Boolean(detailProvider.value),
);
// ADR 0013 迭代二：模型行尾"编辑/删除"与列表头"新增模型"同源门控——当前提供商
// 存在且无任何表单编辑/创建进行中（空闲态）即可用；编辑期间由行级禁用兜底。
const modelActionsIdle = computed(
  () => Boolean(detailProvider.value) && !isEditing.value && !modelCreateOpen.value,
);
// 提供商详情 section 的"编辑"：仅 provider 视图态可发起。
const canEditProvider = computed(
  () => isProviderEntity.value && isViewMode.value && Boolean(detailProvider.value),
);
const isEditingProvider = computed(() => isProviderEntity.value && isEditing.value);
const isEditingModel = computed(() => isModelEntity.value && isEditing.value);
const providerSectionLocked = computed(() => isEditingProvider.value);
const modelSectionLocked = computed(() => isEditingModel.value || modelCreateOpen.value);
const canSaveProvider = computed(
  () =>
    isProviderEntity.value &&
    isEditing.value &&
    !saving.value &&
    Boolean(providerForm.name.trim()) &&
    providerEnabledProtocols.value.length > 0 &&
    providerEnabledProtocols.value.every(
      (protocol) => Boolean(providerForm.endpoints[protocol].baseUrl.trim()),
    ),
);
const canSaveModel = computed(
  () =>
    isModelEntity.value &&
    isEditing.value &&
    !saving.value &&
    Boolean(modelForm.name.trim() && modelForm.model.trim()),
);

// ADR 0013：页头只承载"当前提供商"上下文；实体读写面标题由两个一级折叠区自持
// （提供商详情 / 模型列表），消除旧 editorTitle 与 section 标题的同名重复。
const headerTitle = computed(() => {
  if (isProviderEntity.value && isCreateMode.value) {
    return "新增提供商";
  }

  return detailProvider.value?.name?.trim() || "未命名提供商";
});

const headerDescription =
  "一个提供商可以同时接入多种协议，每种协议单独维护自己的 Base URL 与认证方式。";

function createEndpointRecord(
  endpoints?: ProviderProtocolEndpoint[],
): ProviderFormState["endpoints"] {
  const record: ProviderFormState["endpoints"] = {
    openai: {
      enabled: false,
      baseUrl: defaultBaseUrlFor("openai"),
      authType: "auto",
    },
    anthropic: {
      enabled: false,
      baseUrl: defaultBaseUrlFor("anthropic"),
      authType: "x-api-key",
    },
  };

  for (const endpoint of endpoints ?? []) {
    record[endpoint.protocol] = {
      enabled: endpoint.enabled,
      baseUrl: endpoint.baseUrl,
      authType: endpoint.authType,
    };
  }

  return record;
}

function findProvider(providerId: string) {
  return providers.value.find((provider) => provider.id === providerId) ?? null;
}

function findModel(providerId: string, modelId: string) {
  return (
    providers.value
      .find((provider) => provider.id === providerId)
      ?.models.find((model) => model.id === modelId) ?? null
  );
}

function resetModelActionStates() {
  modelSaveSucceeded.value = false;
  if (modelSaveSuccessTimer) {
    clearTimeout(modelSaveSuccessTimer);
    modelSaveSuccessTimer = null;
  }
}

function setModelSaveSuccess() {
  modelSaveSucceeded.value = true;
  if (modelSaveSuccessTimer) {
    clearTimeout(modelSaveSuccessTimer);
  }
  modelSaveSuccessTimer = setTimeout(() => {
    modelSaveSucceeded.value = false;
    modelSaveSuccessTimer = null;
  }, 1800);
}

function resetProviderForm() {
  providerForm.name = "";
  providerForm.endpoints = createEndpointRecord([
    {
      protocol: "openai",
      enabled: true,
      baseUrl: defaultBaseUrlFor("openai"),
      authType: "auto",
    },
    {
      protocol: "anthropic",
      enabled: false,
      baseUrl: defaultBaseUrlFor("anthropic"),
      authType: "x-api-key",
    },
  ]);
  providerForm.apiKeyValue = "";
}

function fillProviderForm(provider: ProviderConfig) {
  providerForm.name = provider.name;
  providerForm.endpoints = createEndpointRecord(provider.endpoints);
  providerForm.apiKeyValue = provider.apiKeyValue;
}

function resetModelForm() {
  modelForm.id = null;
  modelForm.name = "";
  modelForm.model = "";
  modelForm.contextWindowTokens = "256000";
  modelForm.maxOutputTokens = "64000";
  modelForm.supportsReasoning = false;
  modelForm.supportsImageInput = false;
  modelForm.supportsVideoInput = false;
  modelForm.supportsAudioInput = false;
  modelForm.supportsTextOutput = true;
  modelForm.supportsImageOutput = false;
  modelForm.supportsVideoOutput = false;
  modelForm.supportsAudioOutput = false;
}

function toPositiveIntegerString(value: number | null | undefined) {
  return value && value > 0 ? String(value) : "";
}

function parseOptionalPositiveInteger(value: string) {
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) && parsed > 0 ? Math.trunc(parsed) : null;
}

function getModelCapabilitiesFromForm(): ProviderModelCapabilities {
  return {
    ...createDefaultCapabilities(),
    contextWindowTokens: parseOptionalPositiveInteger(modelForm.contextWindowTokens) ?? 256000,
    supportsReasoning: modelForm.supportsReasoning,
    supportsImageInput: modelForm.supportsImageInput,
    supportsVideoInput: modelForm.supportsVideoInput,
    supportsAudioInput: modelForm.supportsAudioInput,
    supportsTextOutput: modelForm.supportsTextOutput,
    supportsImageOutput: modelForm.supportsImageOutput,
    supportsVideoOutput: modelForm.supportsVideoOutput,
    supportsAudioOutput: modelForm.supportsAudioOutput,
  };
}

function fillModelForm(model: ProviderModelConfig) {
  const capabilities = {
    ...createDefaultCapabilities(),
    ...model.capabilities,
  };
  const userPolicy = resolveModelUserPolicy(model, capabilities);
  modelForm.id = model.id;
  modelForm.name = model.name;
  modelForm.model = model.model;
  modelForm.contextWindowTokens = toPositiveIntegerString(capabilities.contextWindowTokens);
  modelForm.maxOutputTokens = toPositiveIntegerString(userPolicy.maxOutputTokens);
  modelForm.supportsReasoning = capabilities.supportsReasoning;
  modelForm.supportsImageInput = capabilities.supportsImageInput;
  modelForm.supportsVideoInput = capabilities.supportsVideoInput;
  modelForm.supportsAudioInput = capabilities.supportsAudioInput;
  modelForm.supportsTextOutput = capabilities.supportsTextOutput;
  modelForm.supportsImageOutput = capabilities.supportsImageOutput;
  modelForm.supportsVideoOutput = capabilities.supportsVideoOutput;
  modelForm.supportsAudioOutput = capabilities.supportsAudioOutput;
}

// ADR 0013 迭代三：两一级区手风琴互斥——展开其一收起另一；表单编辑中锁定所在区
// 折叠，避免"保存/取消指向不可见表单"的死状态。整行点击触发，动作区 @click.stop。
function toggleProviderSection() {
  if (providerSectionLocked.value) {
    return;
  }
  providerSectionOpen.value = !providerSectionOpen.value;
  if (providerSectionOpen.value) {
    modelSectionOpen.value = false;
  }
}

function toggleModelSection() {
  if (modelSectionLocked.value) {
    return;
  }
  modelSectionOpen.value = !modelSectionOpen.value;
  if (modelSectionOpen.value) {
    providerSectionOpen.value = false;
  }
}

// ADR 0013：模型行点击语义——再次点击已展开行收起并回落提供商视图；否则展开
// 显示"模型配置详情"。任一编辑态下整行点击被守卫拦截（行尾动作已随 v-if 移除），
// 静默丢弃未保存输入的路径在列表内被阻断。
function toggleModelRow(providerId: string, modelId: string) {
  if (isEditingProvider.value || modelSectionLocked.value) {
    return;
  }
  if (expandedModelId.value === modelId) {
    beginViewProvider(providerId);
    return;
  }
  beginViewModel(providerId, modelId);
}

function beginViewProvider(providerId: string) {
  const provider = findProvider(providerId);
  if (!provider) {
    return;
  }

  resetModelActionStates();
  providerStore.selectProvider(providerId);
  expandedModelId.value = null;
  modelCreateOpen.value = false;
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
  expandedModelId.value = modelId;
  modelCreateOpen.value = false;
  editorState.entity = "model";
  editorState.mode = "view";
  editorState.providerId = providerId;
  editorState.modelId = modelId;
}

function beginCreateProvider() {
  resetModelActionStates();
  resetProviderForm();
  // P1 修复（review A）：折叠态进入创建/编辑时自动展开，杜绝"表单不可见+折叠锁死"。
  // 迭代三：手风琴互斥——提供商区展开时收起模型列表区。
  providerSectionOpen.value = true;
  modelSectionOpen.value = false;
  expandedModelId.value = null;
  modelCreateOpen.value = false;
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
  fillProviderForm(provider);
  providerSectionOpen.value = true;
  modelSectionOpen.value = false;
  expandedModelId.value = null;
  modelCreateOpen.value = false;
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
  resetModelForm();
  modelSectionOpen.value = true;
  providerSectionOpen.value = false;
  modelCreateOpen.value = true;
  expandedModelId.value = null;
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
  fillModelForm(model);
  modelSectionOpen.value = true;
  providerSectionOpen.value = false;
  modelCreateOpen.value = false;
  expandedModelId.value = modelId;
  editorState.entity = "model";
  editorState.mode = "edit";
  editorState.providerId = providerId;
  editorState.modelId = modelId;
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

function buildProviderEndpoints(): ProviderProtocolEndpoint[] {
  return endpointOrder.map((protocol) => ({
    protocol,
    enabled: providerForm.endpoints[protocol].enabled,
    baseUrl: providerForm.endpoints[protocol].baseUrl.trim() || defaultBaseUrlFor(protocol),
    authType: providerForm.endpoints[protocol].authType,
  }));
}

async function saveProviderForm() {
  const name = providerForm.name.trim();
  if (!name) {
    return;
  }

  let providerId = editorState.providerId;
  if (editorState.mode === "create") {
    providerId = providerStore.addProvider() ?? null;
  }
  if (!providerId) {
    return;
  }

  const endpoints = buildProviderEndpoints();
  const supportedProtocols = endpoints.filter((item) => item.enabled).map((item) => item.protocol);
  const primaryProtocol = supportedProtocols[0] ?? "openai";
  const primaryEndpoint = endpoints.find((item) => item.protocol === primaryProtocol);

  providerStore.updateProviderField(providerId, "name", name);
  providerStore.updateProviderField(providerId, "supportedProtocols", supportedProtocols);
  providerStore.updateProviderField(providerId, "endpoints", endpoints);
  providerStore.updateProviderField(providerId, "protocol", primaryProtocol);
  providerStore.updateProviderField(
    providerId,
    "baseUrl",
    primaryEndpoint?.baseUrl ?? defaultBaseUrlFor(primaryProtocol),
  );
  providerStore.updateProviderField(providerId, "authType", primaryEndpoint?.authType ?? "auto");
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

  const payloadId = editorState.mode === "edit" && modelForm.id ? modelForm.id : `model-${crypto.randomUUID?.() ?? Date.now()}`;
  const capabilities = getModelCapabilitiesFromForm();
  const userPolicy = resolveModelUserPolicy(
    {
      temperature: 0,
      maxOutputTokens: parseOptionalPositiveInteger(modelForm.maxOutputTokens) ?? 64000,
      reasoningEffort: null,
      reasoningBudgetTokens: null,
    },
    capabilities,
  );

  const savedModel = providerStore.upsertModel(
    editorState.providerId,
    buildProviderModelConfig(
      {
        id: payloadId,
        name,
        model: modelIdValue,
        protocol: detailProvider.value?.supportedProtocols?.[0] ?? "openai",
      },
      resolveCapabilityDeclaration(
        detailProvider.value?.supportedProtocols?.[0] ?? "openai",
        modelIdValue,
        "custom",
        capabilities,
      ),
      userPolicy,
    ),
  );
  if (!savedModel) {
    return;
  }
  providerStore.selectModel(editorState.providerId, savedModel.id);

  await providerStore.saveRegistry();

  if (!providerStore.error) {
    providerStore.notice = editorState.mode === "edit" ? "模型已更新。" : "模型已新增。";
    beginViewModel(editorState.providerId, savedModel.id);
    setModelSaveSuccess();
  }
}

// ADR 0013 迭代二：删除动作移至模型行尾，可对任意行直接触发（编辑锁期间行整体禁用）。
async function removeModelById(providerId: string | null, modelId: string) {
  const targetProviderId = providerId ?? editorState.providerId;
  if (!targetProviderId || !findModel(targetProviderId, modelId)) {
    return;
  }

  const wasActiveModel = isModelEntity.value && editorState.modelId === modelId;
  providerStore.removeModel(targetProviderId, modelId);
  await providerStore.saveRegistry();

  if (providerStore.error) {
    return;
  }

  providerStore.notice = "模型已删除。";
  if (wasActiveModel || expandedModelId.value === modelId) {
    beginViewProvider(targetProviderId);
  }
}

function authTypeLabel(value: ProviderAuthType) {
  switch (value) {
    case "bearer":
      return "Bearer Token";
    case "x-api-key":
      return "x-api-key";
    default:
      return "自动";
  }
}

function providerApiKeySummary(provider: ProviderConfig) {
  if (provider.apiKeyValue?.trim()) {
    return "已填写待保存的新密钥";
  }

  return provider.apiKeyPresent ? "已有已保存密钥" : "未配置";
}

function protocolLabel(protocol: ProviderProtocol) {
  return protocol === "openai" ? "OpenAI 协议" : "Anthropic 协议";
}

function toggleCapability(key: ModelCapabilityToggleKey) {
  modelForm[key] = !modelForm[key];
}

watch(
  providers,
  (providerList) => {
    if (hasInitializedEditor.value) {
      return;
    }

    hasInitializedEditor.value = true;
    const initialProvider = currentProvider.value ?? providerList[0] ?? null;
    if (initialProvider) {
      beginViewProvider(initialProvider.id);
      return;
    }

    beginCreateProvider();
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (modelSaveSuccessTimer) {
    clearTimeout(modelSaveSuccessTimer);
  }
});
</script>

<template>
  <section class="grid h-full min-h-0 gap-3 lg:grid-cols-[290px_minmax(0,1fr)]">
    <aside class="flex min-h-0 flex-col overflow-hidden rounded-[0.55rem] bg-orange-100/50 p-3">
      <div class="flex items-center justify-between gap-2">
        <div>
          <div class="text-sm font-semibold text-stone-900">提供商</div>
          <div class="text-[11px] text-stone-500">管理协议接入与模型挂载</div>
        </div>
        <Button size="sm" variant="ghost" class="shrink-0" data-testid="provider-create-open" @click="beginCreateProvider()">
          <Plus class="mr-1 h-3.5 w-3.5" />
          新增提供商
        </Button>
      </div>

      <!-- ADR 0013：左列为纯选择列表（不可折叠）——点击即选中，右侧双折叠区跟随切换。 -->
      <ScrollArea class="mt-3 min-h-0 flex-1" viewport-class="h-full w-full pr-1">
        <div class="space-y-1.5">
          <button
            v-for="provider in providers"
            :key="provider.id"
            type="button"
            class="flex w-full items-center justify-between gap-2 rounded-[0.35rem] px-2.5 py-2 text-left transition hover:bg-white/74 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
            :class="
              !isCreateMode && detailProvider?.id === provider.id
                ? 'bg-white/78 ring-1 ring-stone-200/70'
                : 'bg-white/30'
            "
            :data-testid="`provider-list-item-${provider.id}`"
            @click="beginViewProvider(provider.id)"
          >
            <span class="min-w-0 truncate text-sm font-medium text-stone-950">
              {{ provider.name || "未命名提供商" }}
            </span>
            <!-- 迭代四：协议与模型数徽标同行尾部显示，不再换行堆叠 -->
            <span class="flex shrink-0 items-center gap-1">
              <span
                v-for="protocol in provider.supportedProtocols"
                :key="protocol"
                class="rounded-[0.2rem] bg-white/40 px-1.5 py-[1px] text-[9px] leading-[1.4] text-stone-400/80"
              >
                {{ protocol === "openai" ? "OpenAI" : "Anthropic" }}
              </span>
              <span class="inline-flex items-center justify-center rounded-full bg-stone-200/60 px-1.5 text-[9px] font-medium leading-[1.4] text-stone-400">
                {{ provider.models.length }}
              </span>
            </span>
          </button>
        </div>
      </ScrollArea>
    </aside>

    <section class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.55rem] px-3 py-3.5 sm:px-4">
      <div v-if="loading" class="rounded-[0.45rem] bg-white/70 px-3.5 py-3 text-sm text-stone-500">
        正在读取配置...
      </div>

      <template v-else>
        <div class="pb-2">
          <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">{{ headerTitle }}</h2>
          <p class="mt-1 text-[12px] leading-5 text-stone-500">{{ headerDescription }}</p>
        </div>

        <ScrollArea class="min-h-0 flex-1" viewport-class="h-full w-full pr-1">
          <div class="space-y-2 pb-1">
            <!-- 保存结果反馈置于区块顶部：动作条在各 section 头部，反馈若沉底会滚出视口。 -->
            <div v-if="notice" class="rounded-[0.45rem] bg-amber-50/85 px-3.5 py-3 text-sm text-amber-950">
              {{ notice }}
            </div>
            <div v-if="error" class="rounded-[0.45rem] bg-rose-50/90 px-3.5 py-3 text-sm text-rose-800">
              {{ error }}
            </div>

            <!-- ─── 一级折叠区 1/2：提供商详情（ADR 0013） ───────────────── -->
            <section
              class="rounded-[0.55rem] bg-white/72 px-3.5 py-2"
              data-testid="provider-detail-section"
            >
              <!-- 迭代三：整行为折叠 trigger（含尾部动作区外的全部区域），hover 背景包裹整行；
                   动作区 @click.stop 特殊处理。 -->
              <div
                class="-mx-1 flex min-h-[1.75rem] cursor-pointer items-center justify-between gap-2 rounded-[0.35rem] px-1"
                :title="isEditingProvider ? '提供商编辑中，暂不可折叠' : undefined"
                data-testid="provider-detail-header"
                @click="toggleProviderSection()"
              >
                <button
                  type="button"
                  class="flex min-w-0 flex-1 items-center gap-1 bg-transparent text-left outline-none"
                  :aria-expanded="providerSectionOpen"
                  aria-controls="provider-detail-body"
                  :aria-label="providerSectionOpen ? '收起提供商详情' : '展开提供商详情'"
                  data-testid="provider-detail-toggle"
                >
                  <ChevronDown
                    class="h-3.5 w-3.5 shrink-0 text-stone-400 transition-transform duration-200 motion-reduce:transition-none"
                    :class="providerSectionOpen ? 'rotate-180' : ''"
                  />
                  <span class="truncate text-sm font-semibold text-stone-950">提供商详情</span>
                </button>

                <!-- 动作区：阻断行点击冒泡；编辑/删除为弱化纯图标（自身 hover 增强）。 -->
                <div class="flex shrink-0 items-center gap-0.5" @click.stop>
                  <Tooltip v-if="canEditProvider && detailProvider" text="编辑" side="top">
                    <button
                      type="button"
                      :class="ICON_ACTION_CLASS"
                      aria-label="编辑提供商"
                      data-testid="provider-edit-open"
                      @click="beginEditProvider(detailProvider.id)"
                    >
                      <Pencil class="h-3.5 w-3.5" />
                    </button>
                  </Tooltip>
                  <Button v-if="isEditingProvider" size="sm" variant="ghost" data-testid="provider-edit-cancel" @click="cancelEditing()">取消</Button>
                  <ConfirmPopover
                    v-if="canDeleteProvider"
                    :title="`删除提供商「${detailProvider?.name || detailProvider?.id || '当前提供商'}」？`"
                    description="此操作不可撤销。"
                    side="bottom"
                    align="end"
                    @confirm="removeCurrentProvider()"
                  >
                    <Tooltip text="删除" side="top">
                      <button type="button" :class="ICON_ACTION_CLASS" aria-label="删除提供商">
                        <Trash2 class="h-3.5 w-3.5" />
                      </button>
                    </Tooltip>
                  </ConfirmPopover>
                  <Button v-if="canSaveProvider" size="sm" variant="secondary" data-testid="provider-save" @click="saveProviderForm()">
                    <Save class="mr-1 h-4 w-4" />
                    {{ saving ? "保存中..." : "保存" }}
                  </Button>
                </div>
              </div>

              <!-- 平滑折叠：grid-rows 0fr↔1fr 过渡。行高用内联 style 表达，避免依赖
                   Tailwind 任意值类的生成时机；内容常挂载避免布局突跳。 -->
              <div
                id="provider-detail-body"
                class="grid overflow-hidden transition-[grid-template-rows] duration-300 ease-out motion-reduce:transition-none"
                :style="{ gridTemplateRows: providerSectionOpen ? '1fr' : '0fr' }"
                :aria-hidden="!providerSectionOpen"
                :data-open="providerSectionOpen ? 'true' : 'false'"
                data-testid="provider-detail-body"
              >
                <div class="min-h-0">
                  <div class="pt-2">
                    <div class="config-form space-y-2">
                  <!-- P1 修复（review A）：守卫用 isEditingProvider 而非全局 isEditing，
                       防止模型编辑/新增态把陈旧 providerForm 泄漏渲染进提供商区。 -->
                  <template v-if="isEditingProvider">
                <div class="grid gap-3">
                  <label class="flex items-center gap-2 text-[11px] text-stone-500"><span class="shrink-0">提供商名称</span><Input :model-value="providerForm.name" placeholder="例如：OpenRouter" @update:model-value="providerForm.name = $event"  class="min-w-0 flex-1" /></label>

                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                    <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                      协议入口
                      <InfoTip text="同一个提供商可以同时开启 OpenAI 和 Anthropic 协议；每种协议独立填写自己的 Base URL 和认证方式。" />
                    </div>

                    <div class="mt-3 grid gap-3 xl:grid-cols-2">
                      <div
                        v-for="protocol in endpointOrder"
                        :key="protocol"
                        class="rounded-[0.45rem] bg-stone-100/75 px-3 py-3"
                      >
                        <div class="flex items-center justify-between gap-3">
                          <div>
                            <div class="text-[13px] font-medium text-stone-900">{{ protocolLabel(protocol) }}</div>
                            <div class="text-[11px] text-stone-500">单独配置 endpoint</div>
                          </div>
                          <button
                            type="button"
                            class="rounded-full px-2.5 py-1 text-[11px] transition"
                            :class="providerForm.endpoints[protocol].enabled ? 'bg-stone-900 text-white' : 'bg-white text-stone-600'"
                            @click="providerForm.endpoints[protocol].enabled = !providerForm.endpoints[protocol].enabled"
                          >
                            {{ providerForm.endpoints[protocol].enabled ? "已启用" : "未启用" }}
                          </button>
                        </div>

                        <div class="mt-1.5 grid gap-x-4 gap-y-1 sm:grid-cols-2">
                          <label class="space-y-1 text-[11px] text-stone-500">
                            <span>Base URL</span>
                            <Input
                              :model-value="providerForm.endpoints[protocol].baseUrl"
                              :disabled="!providerForm.endpoints[protocol].enabled"
                              :placeholder="defaultBaseUrlFor(protocol)"
                              @update:model-value="providerForm.endpoints[protocol].baseUrl = $event"
                            />
                          </label>

                          <label class="space-y-1 text-[11px] text-stone-500">
                            <span>认证方式</span>
                            <select
                              :value="providerForm.endpoints[protocol].authType"
                              class="config-select"
                              :disabled="!providerForm.endpoints[protocol].enabled"
                              @change="providerForm.endpoints[protocol].authType = ($event.target as HTMLSelectElement).value as ProviderAuthType"
                            >
                              <option value="auto">自动</option>
                              <option value="bearer">Bearer Token</option>
                              <option value="x-api-key">x-api-key</option>
                            </select>
                          </label>
                        </div>
                      </div>
                    </div>
                  </section>
                </div>

                <div class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="flex items-center gap-1.5 text-[13px] font-medium text-stone-900">
                    API Key
                    <Shield class="h-3.5 w-3.5 text-stone-500" />
                    <InfoTip text="密钥仍按提供商维度保存到应用密钥存储；providers.json 不保存敏感明文。" />
                  </div>
                  <div class="mt-1.5">
                    <label class="flex items-center gap-2 text-[11px] text-stone-500">
                      <span class="shrink-0">当前密钥</span>
                      <Input
                        class="min-w-0 flex-1"
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
                <!-- 迭代四：字段名与值同行、按长度两栏排布、行距减半。 -->
                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="text-sm font-medium text-stone-900">基础信息</div>
                  <dl class="mt-1.5 grid gap-x-4 gap-y-1 text-[13px] leading-5 sm:grid-cols-2">
                    <div class="flex min-w-0 items-baseline gap-2">
                      <dt class="shrink-0 text-[11px] uppercase tracking-[0.16em] text-stone-400">名称</dt>
                      <dd class="min-w-0 truncate text-stone-900">{{ detailProvider.name || "未命名提供商" }}</dd>
                    </div>
                    <div class="flex min-w-0 items-baseline gap-2">
                      <dt class="shrink-0 text-[11px] uppercase tracking-[0.16em] text-stone-400">已启用协议</dt>
                      <dd class="min-w-0 truncate text-stone-900">{{ detailProvider.supportedProtocols.join(" / ") }}</dd>
                    </div>
                  </dl>
                </section>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                    密钥与环境
                    <Shield class="h-3.5 w-3.5 text-stone-500" />
                  </div>
                  <dl class="mt-1.5 grid gap-x-4 gap-y-1 text-[13px] leading-5 sm:grid-cols-2">
                    <div class="flex min-w-0 items-baseline gap-2">
                      <dt class="shrink-0 text-[11px] uppercase tracking-[0.16em] text-stone-400">环境变量名</dt>
                      <dd class="min-w-0 truncate text-stone-900">{{ detailProvider.apiKeyEnvVar }}</dd>
                    </div>
                    <div class="flex min-w-0 items-baseline gap-2">
                      <dt class="shrink-0 text-[11px] uppercase tracking-[0.16em] text-stone-400">密钥状态</dt>
                      <dd class="min-w-0 truncate text-stone-900">{{ providerApiKeySummary(detailProvider) }}</dd>
                    </div>
                  </dl>
                </section>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="text-sm font-medium text-stone-900">协议入口</div>
                  <div class="mt-1.5 space-y-1">
                    <div
                      v-for="endpoint in detailProvider.endpoints.filter((item) => item.enabled)"
                      :key="endpoint.protocol"
                      class="flex min-w-0 items-baseline justify-between gap-3 rounded-[0.35rem] bg-stone-100/75 px-2.5 py-1.5"
                    >
                      <span class="flex shrink-0 items-baseline gap-2">
                        <span class="text-[12px] font-medium text-stone-900">{{ protocolLabel(endpoint.protocol) }}</span>
                        <span class="text-[11px] text-stone-500">{{ authTypeLabel(endpoint.authType) }}</span>
                      </span>
                      <span class="min-w-0 truncate text-[12px] text-stone-500">{{ endpoint.baseUrl }}</span>
                    </div>
                  </div>
                </section>
              </template>
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <!-- ─── 一级折叠区 2/2：模型列表（create-provider 模式下隐藏，ADR 0013） ─── -->
            <section
              v-if="!isCreateMode || !isProviderEntity"
              class="rounded-[0.55rem] bg-white/72 px-3.5 py-2"
              data-testid="model-list-section"
            >
              <div
                class="-mx-1 flex min-h-[1.75rem] cursor-pointer items-center justify-between gap-2 rounded-[0.35rem] px-1"
                :title="modelSectionLocked ? '模型表单填写中，暂不可折叠' : undefined"
                data-testid="model-list-header"
                @click="toggleModelSection()"
              >
                <button
                  type="button"
                  class="flex min-w-0 flex-1 items-center gap-1 bg-transparent text-left outline-none"
                  :aria-expanded="modelSectionOpen"
                  aria-controls="model-list-body"
                  :aria-label="modelSectionOpen ? '收起模型列表' : '展开模型列表'"
                  data-testid="model-list-toggle"
                >
                  <ChevronDown
                    class="h-3.5 w-3.5 shrink-0 text-stone-400 transition-transform duration-200 motion-reduce:transition-none"
                    :class="modelSectionOpen ? 'rotate-180' : ''"
                  />
                  <span class="truncate text-sm font-semibold text-stone-950">模型列表</span>
                </button>

                <div class="flex shrink-0 items-center gap-0.5" @click.stop>
                  <Tooltip v-if="modelActionsIdle && detailProvider" text="新增模型" side="top">
                    <button
                      type="button"
                      :class="ICON_ACTION_CLASS"
                      aria-label="新增模型"
                      data-testid="model-create-open"
                      @click="beginCreateModel(detailProvider.id)"
                    >
                      <Plus class="h-3.5 w-3.5" />
                    </button>
                  </Tooltip>
                </div>
              </div>

              <div
                id="model-list-body"
                class="grid overflow-hidden transition-[grid-template-rows] duration-300 ease-out motion-reduce:transition-none"
                :style="{ gridTemplateRows: modelSectionOpen ? '1fr' : '0fr' }"
                :aria-hidden="!modelSectionOpen"
                :data-open="modelSectionOpen ? 'true' : 'false'"
                data-testid="model-list-body"
              >
                <div class="min-h-0">
                  <div class="pt-2">
                <!-- 新增模型表单卡（编辑态复用同一份表单标记，仅渲染位置不同；两份需同步维护）。 -->
                <div
                  v-if="modelCreateOpen"
                  class="mb-3 rounded-[0.45rem] bg-stone-100/70 px-3.5 py-3"
                  data-testid="model-create-card"
                >
                  <div class="flex items-center justify-between gap-2 pb-1">
                    <div class="text-sm font-medium text-stone-900">新增模型</div>
                    <div class="flex gap-1.5">
                      <Button size="sm" variant="ghost" data-testid="model-create-cancel" @click="cancelEditing()">取消</Button>
                      <Button v-if="canSaveModel" size="sm" variant="secondary" data-testid="model-create-save" @click="saveModelForm()">
                        <Save class="mr-1 h-4 w-4" />
                        {{ saving ? "保存中..." : "保存" }}
                      </Button>
                    </div>
                  </div>

                  <div class="config-form space-y-2">
                    <div class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                      <label class="flex items-center gap-2 text-[11px] text-stone-500"><span class="shrink-0">名称</span><Input :model-value="modelForm.name" placeholder="例如：Claude Sonnet 4" @update:model-value="modelForm.name = $event"  class="min-w-0 flex-1" /></label>

                      <label class="flex items-center gap-2 text-[11px] text-stone-500"><span class="shrink-0">模型 ID</span><Input :model-value="modelForm.model" placeholder="例如：claude-sonnet-4-20250514" @update:model-value="modelForm.model = $event"  class="min-w-0 flex-1" /></label>
                    </div>

                    <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                      <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                        模型能力
                        <InfoTip text="能力保持输入 / 输出两行；图标悬停可查看说明，明暗表示启用与未启用。" />
                      </div>
                      <div class="mt-1.5 grid gap-x-4 gap-y-1 sm:grid-cols-2">
                        <div>
                          <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输入</div>
                          <div class="flex flex-wrap gap-2">
                            <Tooltip
                              v-for="option in inputCapabilityOptions"
                              :key="option.key"
                              :text="option.label"
                              side="top"
                            >
                              <button
                                type="button"
                                class="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-[0.55rem] transition"
                                :class="
                                  modelForm[option.key]
                                    ? 'bg-stone-900 text-stone-50'
                                    : 'bg-stone-100/90 text-stone-500 hover:bg-stone-200/80'
                                "
                                :aria-label="option.label"
                                :aria-pressed="modelForm[option.key]"
                                @click="toggleCapability(option.key)"
                              >
                                <component :is="option.icon" class="h-4 w-4" />
                              </button>
                            </Tooltip>
                          </div>
                        </div>

                        <div>
                          <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输出</div>
                          <div class="flex flex-wrap gap-2">
                            <Tooltip
                              v-for="option in outputCapabilityOptions"
                              :key="option.key"
                              :text="option.label"
                              side="top"
                            >
                              <button
                                type="button"
                                class="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-[0.55rem] transition"
                                :class="
                                  modelForm[option.key]
                                    ? 'bg-stone-900 text-stone-50'
                                    : 'bg-stone-100/90 text-stone-500 hover:bg-stone-200/80'
                                "
                                :aria-label="option.label"
                                :aria-pressed="modelForm[option.key]"
                                @click="toggleCapability(option.key)"
                              >
                                <component :is="option.icon" class="h-4 w-4" />
                              </button>
                            </Tooltip>
                          </div>
                        </div>
                      </div>
                    </section>

                    <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                      <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                        模型参数
                        <InfoTip text="上下文与最大输出以 K/M 显示；徽标为常规项，点选即填入。" />
                      </div>
                      <div class="mt-3 grid gap-3 xl:grid-cols-2">
                        <div class="space-y-1 text-[11px] text-stone-500">
                          <div class="flex items-center gap-2"><span class="shrink-0">上下文长度</span><Input :model-value="modelForm.contextWindowTokens" type="number" @update:model-value="modelForm.contextWindowTokens = $event"  class="min-w-0 flex-1" /></div>
                          <div class="flex flex-wrap gap-1 pt-0.5">
                            <button
                              v-for="preset in CONTEXT_TOKEN_PRESETS"
                              :key="preset"
                              type="button"
                              class="rounded-full bg-white/80 px-2 py-[2px] text-[10px] leading-[1.4] text-stone-500 ring-1 ring-stone-200/70 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                              :data-testid="`context-preset-${formatTokenUnit(preset, '')}`"
                              @click="modelForm.contextWindowTokens = String(preset)"
                            >
                              {{ formatTokenUnit(preset, "") }}
                            </button>
                          </div>
                        </div>
                        <div class="space-y-1 text-[11px] text-stone-500">
                          <div class="flex items-center gap-2"><span class="shrink-0">最大输出长度</span><Input :model-value="modelForm.maxOutputTokens" type="number" @update:model-value="modelForm.maxOutputTokens = $event"  class="min-w-0 flex-1" /></div>
                          <div class="flex flex-wrap gap-1 pt-0.5">
                            <button
                              v-for="preset in MAX_OUTPUT_TOKEN_PRESETS"
                              :key="preset"
                              type="button"
                              class="rounded-full bg-white/80 px-2 py-[2px] text-[10px] leading-[1.4] text-stone-500 ring-1 ring-stone-200/70 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                              :data-testid="`output-preset-${formatTokenUnit(preset, '')}`"
                              @click="modelForm.maxOutputTokens = String(preset)"
                            >
                              {{ formatTokenUnit(preset, "") }}
                            </button>
                          </div>
                        </div>
                      </div>
                    </section>
                  </div>
                </div>

                <p
                  v-if="detailProvider && detailProvider.models.length === 0 && !modelCreateOpen"
                  class="py-2 text-[12px] leading-5 text-stone-500"
                  data-testid="model-list-empty"
                >
                  该提供商暂无模型；点击右上角"新增模型"开始接入。
                </p>

                <div class="space-y-1.5">
                  <div
                    v-for="model in detailProvider?.models ?? []"
                    :key="model.id"
                    class="overflow-hidden rounded-[0.45rem] bg-stone-100/50"
                  >
                    <!-- 迭代三：整行为折叠 trigger（hover 背景包裹含行尾动作的整行），
                         行尾动作区 @click.stop 特殊处理；chevron 为纯指示器。 -->
                    <div
                      class="flex cursor-pointer items-center justify-between gap-2 rounded-[0.35rem] px-2 py-1.5 transition-colors hover:bg-white/74 motion-reduce:transition-none"
                      :data-testid="`model-list-item-${model.id}`"
                      @click="toggleModelRow(detailProvider!.id, model.id)"
                    >
                      <button
                        type="button"
                        class="flex min-w-0 flex-1 items-center rounded-[0.35rem] px-1 py-0.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
                        :aria-expanded="expandedModelId === model.id"
                        :aria-controls="`model-detail-${model.id}`"
                      >
                        <span class="min-w-0">
                          <span class="block truncate text-[12px] font-medium text-stone-800">
                            {{ model.name || "未命名模型" }}
                          </span>
                          <span class="mt-0.5 block truncate text-[11px] text-stone-500">
                            {{ model.protocol || detailProvider?.protocol }} · {{ model.model || "未填写模型 ID" }}
                          </span>
                        </span>
                      </button>

                      <div class="flex shrink-0 items-center gap-0.5" @click.stop>
                        <Tooltip v-if="modelActionsIdle" text="编辑" side="top">
                          <button
                            type="button"
                            :class="ICON_ACTION_CLASS"
                            aria-label="编辑模型"
                            :data-testid="`model-row-edit-${model.id}`"
                            @click="beginEditModel(detailProvider!.id, model.id)"
                          >
                            <Pencil class="h-3.5 w-3.5" />
                          </button>
                        </Tooltip>
                        <ConfirmPopover
                          v-if="modelActionsIdle"
                          :title="`删除模型「${model.name || model.model || '未命名模型'}」？`"
                          description="此操作不可撤销。"
                          side="bottom"
                          align="end"
                          @confirm="removeModelById(detailProvider!.id, model.id)"
                        >
                          <Tooltip text="删除" side="top">
                            <button
                              type="button"
                              :class="ICON_ACTION_CLASS"
                              aria-label="删除模型"
                              :data-testid="`model-row-delete-${model.id}`"
                            >
                              <Trash2 class="h-3.5 w-3.5" />
                            </button>
                          </Tooltip>
                        </ConfirmPopover>
                        <ChevronDown
                          aria-hidden="true"
                          class="h-3.5 w-3.5 shrink-0 text-stone-400 transition-transform duration-200 motion-reduce:transition-none"
                          :class="expandedModelId === model.id ? 'rotate-180' : ''"
                        />
                      </div>
                    </div>

                    <div
                      :id="`model-detail-${model.id}`"
                      class="grid overflow-hidden transition-[grid-template-rows] duration-300 ease-out motion-reduce:transition-none"
                      :style="{ gridTemplateRows: expandedModelId === model.id ? '1fr' : '0fr' }"
                      :aria-hidden="expandedModelId !== model.id"
                      :data-open="expandedModelId === model.id ? 'true' : 'false'"
                      :data-testid="`model-detail-${model.id}`"
                    >
                      <div class="min-h-0">
                        <div
                          class="px-3 pb-3 pt-3"
                          :class="expandedModelId === model.id ? 'border-t border-stone-200/60' : ''"
                        >
                      <div
                        v-if="isEditingModel && editorState.modelId === model.id"
                        class="flex justify-end gap-1.5 pb-2"
                      >
                        <Button
                          size="sm"
                          variant="ghost"
                          :data-testid="`model-edit-cancel-${model.id}`"
                          @click="cancelEditing()"
                        >取消</Button>
                        <Button
                          v-if="canSaveModel"
                          size="sm"
                          variant="secondary"
                          :data-testid="`model-edit-save-${model.id}`"
                          @click="saveModelForm()"
                        >
                          <Check v-if="modelSaveSucceeded && !saving" class="mr-1 h-4 w-4" />
                          <Save v-else class="mr-1 h-4 w-4" />
                          {{ saving ? "保存中..." : modelSaveSucceeded ? "已保存" : "保存" }}
                        </Button>
                      </div>

                      <div class="config-form space-y-2">
                        <template v-if="isEditingModel && editorState.modelId === model.id">
                <div class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                  <label class="flex items-center gap-2 text-[11px] text-stone-500"><span class="shrink-0">名称</span><Input :model-value="modelForm.name" placeholder="例如：Claude Sonnet 4" @update:model-value="modelForm.name = $event"  class="min-w-0 flex-1" /></label>

                  <label class="flex items-center gap-2 text-[11px] text-stone-500"><span class="shrink-0">模型 ID</span><Input :model-value="modelForm.model" placeholder="例如：claude-sonnet-4-20250514" @update:model-value="modelForm.model = $event"  class="min-w-0 flex-1" /></label>
                </div>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                    模型能力
                    <InfoTip text="能力保持输入 / 输出两行；图标悬停可查看说明，明暗表示启用与未启用。" />
                  </div>
                  <div class="mt-1.5 grid gap-x-4 gap-y-1 sm:grid-cols-2">
                    <div>
                      <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输入</div>
                      <div class="flex flex-wrap gap-2">
                        <Tooltip
                          v-for="option in inputCapabilityOptions"
                          :key="option.key"
                          :text="option.label"
                          side="top"
                        >
                          <button
                            type="button"
                            class="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-[0.55rem] transition"
                            :class="
                              modelForm[option.key]
                                ? 'bg-stone-900 text-stone-50'
                                : 'bg-stone-100/90 text-stone-500 hover:bg-stone-200/80'
                            "
                            :aria-label="option.label"
                            :aria-pressed="modelForm[option.key]"
                            @click="toggleCapability(option.key)"
                          >
                            <component :is="option.icon" class="h-4 w-4" />
                          </button>
                        </Tooltip>
                      </div>
                    </div>

                    <div>
                      <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输出</div>
                      <div class="flex flex-wrap gap-2">
                        <Tooltip
                          v-for="option in outputCapabilityOptions"
                          :key="option.key"
                          :text="option.label"
                          side="top"
                        >
                          <button
                            type="button"
                            class="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-[0.55rem] transition"
                            :class="
                              modelForm[option.key]
                                ? 'bg-stone-900 text-stone-50'
                                : 'bg-stone-100/90 text-stone-500 hover:bg-stone-200/80'
                            "
                            :aria-label="option.label"
                            :aria-pressed="modelForm[option.key]"
                            @click="toggleCapability(option.key)"
                          >
                            <component :is="option.icon" class="h-4 w-4" />
                          </button>
                        </Tooltip>
                      </div>
                    </div>
                  </div>
                </section>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                    模型参数
                    <InfoTip text="上下文与最大输出以 K/M 显示；徽标为常规项，点选即填入。" />
                  </div>
                  <div class="mt-3 grid gap-3 xl:grid-cols-2">
                    <div class="space-y-1 text-[11px] text-stone-500">
                      <div class="flex items-center gap-2"><span class="shrink-0">上下文长度</span><Input :model-value="modelForm.contextWindowTokens" type="number" @update:model-value="modelForm.contextWindowTokens = $event"  class="min-w-0 flex-1" /></div>
                      <div class="flex flex-wrap gap-1 pt-0.5">
                        <button
                          v-for="preset in CONTEXT_TOKEN_PRESETS"
                          :key="preset"
                          type="button"
                          class="rounded-full bg-white/80 px-2 py-[2px] text-[10px] leading-[1.4] text-stone-500 ring-1 ring-stone-200/70 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                          :data-testid="`context-preset-${formatTokenUnit(preset, '')}`"
                          @click="modelForm.contextWindowTokens = String(preset)"
                        >
                          {{ formatTokenUnit(preset, "") }}
                        </button>
                      </div>
                    </div>
                    <div class="space-y-1 text-[11px] text-stone-500">
                      <div class="flex items-center gap-2"><span class="shrink-0">最大输出长度</span><Input :model-value="modelForm.maxOutputTokens" type="number" @update:model-value="modelForm.maxOutputTokens = $event"  class="min-w-0 flex-1" /></div>
                      <div class="flex flex-wrap gap-1 pt-0.5">
                        <button
                          v-for="preset in MAX_OUTPUT_TOKEN_PRESETS"
                          :key="preset"
                          type="button"
                          class="rounded-full bg-white/80 px-2 py-[2px] text-[10px] leading-[1.4] text-stone-500 ring-1 ring-stone-200/70 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                          :data-testid="`output-preset-${formatTokenUnit(preset, '')}`"
                          @click="modelForm.maxOutputTokens = String(preset)"
                        >
                          {{ formatTokenUnit(preset, "") }}
                        </button>
                      </div>
                    </div>
                  </div>
                </section>
              </template>

                        <template v-else-if="!isEditingModel && editorState.modelId === model.id && detailModel">
                <!-- 迭代四：字段名与值同行、两栏排布、行距减半。 -->
                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="text-sm font-medium text-stone-900">模型信息</div>
                  <dl class="mt-1.5 grid gap-x-4 gap-y-1 text-[13px] leading-5 sm:grid-cols-2">
                    <div class="flex min-w-0 items-baseline gap-2">
                      <dt class="shrink-0 text-[11px] uppercase tracking-[0.16em] text-stone-400">名称</dt>
                      <dd class="min-w-0 truncate text-stone-900">{{ detailModel.name || "未命名模型" }}</dd>
                    </div>
                    <div class="flex min-w-0 items-baseline gap-2 sm:col-span-2">
                      <dt class="shrink-0 text-[11px] uppercase tracking-[0.16em] text-stone-400">模型 ID</dt>
                      <dd class="min-w-0 break-words text-stone-900">{{ detailModel.model || "未填写模型 ID" }}</dd>
                    </div>
                  </dl>
                </section>

                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                    <div class="text-sm font-medium text-stone-900">模型能力</div>
                    <div class="mt-1.5 grid gap-x-4 gap-y-1 sm:grid-cols-2">
                      <div>
                        <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输入</div>
                        <div class="flex flex-wrap gap-1.5">
                          <Tooltip
                            v-for="option in inputCapabilityOptions"
                            :key="option.key"
                            :text="option.label"
                            side="top"
                          >
                            <div
                              class="inline-flex h-7 w-7 cursor-default items-center justify-center rounded-[0.55rem]"
                              :class="
                                ({ ...createDefaultCapabilities(), ...detailModel.capabilities })[option.key]
                                  ? 'bg-stone-900 text-stone-50'
                                  : 'bg-stone-100/90 text-stone-500'
                              "
                            >
                              <component :is="option.icon" class="h-4 w-4" />
                            </div>
                          </Tooltip>
                        </div>
                      </div>

                      <div>
                        <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输出</div>
                        <div class="flex flex-wrap gap-1.5">
                          <Tooltip
                            v-for="option in outputCapabilityOptions"
                            :key="option.key"
                            :text="option.label"
                            side="top"
                          >
                            <div
                              class="inline-flex h-7 w-7 cursor-default items-center justify-center rounded-[0.55rem]"
                              :class="
                                ({ ...createDefaultCapabilities(), ...detailModel.capabilities })[option.key]
                                  ? 'bg-stone-900 text-stone-50'
                                  : 'bg-stone-100/90 text-stone-500'
                              "
                            >
                              <component :is="option.icon" class="h-4 w-4" />
                            </div>
                          </Tooltip>
                        </div>
                      </div>
                    </div>
                  </section>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <div class="text-sm font-medium text-stone-900">模型参数</div>
                  <div class="mt-3 grid gap-3 xl:grid-cols-2">
                    <div>
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">上下文长度</div>
                      <div class="mt-1 rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5 text-[13px] text-stone-900">
                        {{ formatTokenUnit(detailModel.capabilities.contextWindowTokens, "256K") }}
                      </div>
                    </div>
                    <div>
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">最大输出长度</div>
                      <div class="mt-1 rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5 text-[13px] text-stone-900">
                        {{ formatTokenUnit(detailModel.maxOutputTokens, "64K") }}
                      </div>
                    </div>
                  </div>
                </section>
              </template>
                      </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
                  </div>
                </div>
              </div>
            </section>
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
  transition: background-color 160ms ease;
}

.config-select:focus {
  outline: none;
  background: rgb(255 255 255 / 0.98);
}
</style>
