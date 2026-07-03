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

const openProviderId = ref<string | null>(null);
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
const canDeleteModel = computed(
  () =>
    isModelEntity.value &&
    !isCreateMode.value &&
    Boolean(detailProvider.value && detailModel.value),
);
const canCreateModel = computed(
  () => isProviderEntity.value && !isEditing.value && Boolean(detailProvider.value),
);
const canEditCurrent = computed(() => isViewMode.value && Boolean(detailProvider.value));
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

const editorTitle = computed(() => {
  if (isProviderEntity.value) {
    if (isCreateMode.value) {
      return "新增提供商";
    }

    return isEditing.value ? "编辑提供商" : "提供商详情";
  }

  if (isCreateMode.value) {
    return "新增模型";
  }

  return isEditing.value ? "编辑模型" : "模型详情";
});

const editorDescription = computed(() => {
  if (isProviderEntity.value) {
    return "一个提供商可以同时接入多种协议，每种协议单独维护自己的 Base URL 与认证方式。";
  }

  return "模型只保留核心能力定义与常用参数，避免能力预设和低频参数干扰。";
});

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

function numberLabel(value: number | null | undefined, fallback: string) {
  return value && value > 0 ? `${value.toLocaleString()} tokens` : fallback;
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
      openProviderId.value = initialProvider.id;
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
                <div class="mt-0.5 flex flex-wrap items-center gap-1 pl-6">
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
                  <div class="mt-0.5 truncate text-[11px] text-stone-500">
                    {{ provider.supportedProtocols.length }} 个协议入口
                  </div>
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
                      {{ model.protocol || provider.protocol }} · {{ model.model || "未填写模型 ID" }}
                    </div>
                  </div>
                </button>
              </div>
            </div>
          </section>
        </div>
      </ScrollArea>
    </aside>

    <section class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.55rem] px-3 py-3.5 sm:px-4">
      <div v-if="loading" class="rounded-[0.45rem] bg-white/70 px-3.5 py-3 text-sm text-stone-500">
        正在读取配置...
      </div>

      <template v-else>
        <div class="flex flex-wrap items-start justify-between gap-2.5 pb-3">
          <div>
            <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">{{ editorTitle }}</h2>
            <p class="mt-1 text-[12px] leading-5 text-stone-500">{{ editorDescription }}</p>
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
            <Button v-if="isEditing" size="sm" variant="ghost" @click="cancelEditing()">取消</Button>
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
                <div class="grid gap-3">
                  <label class="space-y-1 text-[11px] text-stone-500">
                    <span>提供商名称</span>
                    <Input :model-value="providerForm.name" placeholder="例如：OpenRouter" @update:model-value="providerForm.name = $event" />
                  </label>

                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
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

                        <div class="mt-3 space-y-3">
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

                <div class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="flex items-center gap-1.5 text-[13px] font-medium text-stone-900">
                    API Key
                    <Shield class="h-3.5 w-3.5 text-stone-500" />
                    <InfoTip text="密钥仍按提供商维度保存到应用密钥存储；providers.json 不保存敏感明文。" />
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
                        <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">已启用协议</div>
                        <div class="mt-1 text-stone-900">{{ detailProvider.supportedProtocols.join(" / ") }}</div>
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

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="text-sm font-medium text-stone-900">协议入口</div>
                  <div class="mt-2.5 grid gap-2.5 xl:grid-cols-2">
                    <div
                      v-for="endpoint in detailProvider.endpoints.filter((item) => item.enabled)"
                      :key="endpoint.protocol"
                      class="rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5"
                    >
                      <div class="text-[13px] font-medium text-stone-900">{{ protocolLabel(endpoint.protocol) }}</div>
                      <div class="mt-1 break-words text-[12px] text-stone-500">{{ endpoint.baseUrl }}</div>
                      <div class="mt-1 text-[11px] text-stone-500">{{ authTypeLabel(endpoint.authType) }}</div>
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
                    <Input :model-value="modelForm.name" placeholder="例如：Claude Sonnet 4" @update:model-value="modelForm.name = $event" />
                  </label>

                  <label class="space-y-1 text-[11px] text-stone-500">
                    <span>模型 ID</span>
                    <Input :model-value="modelForm.model" placeholder="例如：claude-sonnet-4-20250514" @update:model-value="modelForm.model = $event" />
                  </label>
                </div>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                    模型能力
                    <InfoTip text="能力保持输入 / 输出两行；图标悬停可查看说明，明暗表示启用与未启用。" />
                  </div>
                  <div class="mt-3 grid gap-3">
                    <div>
                      <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输入</div>
                      <div class="flex flex-wrap gap-2">
                        <button
                          v-for="option in inputCapabilityOptions"
                          :key="option.key"
                          type="button"
                          class="inline-flex cursor-pointer items-center gap-2 rounded-[0.65rem] px-3 py-2 text-[12px] transition"
                          :class="
                            modelForm[option.key]
                              ? 'bg-stone-900 text-stone-50'
                              : 'bg-stone-100/90 text-stone-500 hover:bg-stone-200/80'
                          "
                          @click="toggleCapability(option.key)"
                        >
                          <component :is="option.icon" class="h-4 w-4" />
                          <span>{{ option.label }}</span>
                        </button>
                      </div>
                    </div>

                    <div>
                      <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输出</div>
                      <div class="flex flex-wrap gap-2">
                        <button
                          v-for="option in outputCapabilityOptions"
                          :key="option.key"
                          type="button"
                          class="inline-flex cursor-pointer items-center gap-2 rounded-[0.65rem] px-3 py-2 text-[12px] transition"
                          :class="
                            modelForm[option.key]
                              ? 'bg-stone-900 text-stone-50'
                              : 'bg-stone-100/90 text-stone-500 hover:bg-stone-200/80'
                          "
                          @click="toggleCapability(option.key)"
                        >
                          <component :is="option.icon" class="h-4 w-4" />
                          <span>{{ option.label }}</span>
                        </button>
                      </div>
                    </div>
                  </div>
                </section>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="flex items-center gap-2 text-sm font-medium text-stone-900">
                    模型参数
                    <InfoTip text="仅保留上下文长度和最大输出长度，两项保持一行展示。" />
                  </div>
                  <div class="mt-3 grid gap-3 xl:grid-cols-2">
                    <label class="space-y-1 text-[11px] text-stone-500">
                      <span>上下文长度</span>
                      <Input :model-value="modelForm.contextWindowTokens" type="number" @update:model-value="modelForm.contextWindowTokens = $event" />
                    </label>
                    <label class="space-y-1 text-[11px] text-stone-500">
                      <span>最大输出长度</span>
                      <Input :model-value="modelForm.maxOutputTokens" type="number" @update:model-value="modelForm.maxOutputTokens = $event" />
                    </label>
                  </div>
                </section>
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
                    </div>
                  </section>

                  <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                    <div class="text-sm font-medium text-stone-900">模型能力</div>
                    <div class="mt-3 grid gap-3">
                      <div>
                        <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输入</div>
                        <div class="flex flex-wrap gap-2">
                          <div
                            v-for="option in inputCapabilityOptions"
                            :key="option.key"
                            class="inline-flex items-center gap-2 rounded-[0.65rem] px-3 py-2 text-[12px]"
                            :class="
                              ({ ...createDefaultCapabilities(), ...detailModel.capabilities })[option.key]
                                ? 'bg-stone-900 text-stone-50'
                                : 'bg-stone-100/90 text-stone-500'
                            "
                          >
                            <component :is="option.icon" class="h-4 w-4" />
                            <span>{{ option.label }}</span>
                          </div>
                        </div>
                      </div>

                      <div>
                        <div class="mb-1 text-[11px] uppercase tracking-[0.16em] text-stone-400">输出</div>
                        <div class="flex flex-wrap gap-2">
                          <div
                            v-for="option in outputCapabilityOptions"
                            :key="option.key"
                            class="inline-flex items-center gap-2 rounded-[0.65rem] px-3 py-2 text-[12px]"
                            :class="
                              ({ ...createDefaultCapabilities(), ...detailModel.capabilities })[option.key]
                                ? 'bg-stone-900 text-stone-50'
                                : 'bg-stone-100/90 text-stone-500'
                            "
                          >
                            <component :is="option.icon" class="h-4 w-4" />
                            <span>{{ option.label }}</span>
                          </div>
                        </div>
                      </div>
                    </div>
                  </section>
                </div>

                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <div class="text-sm font-medium text-stone-900">模型参数</div>
                  <div class="mt-3 grid gap-3 xl:grid-cols-2">
                    <div>
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">上下文长度</div>
                      <div class="mt-1 rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5 text-[13px] text-stone-900">
                        {{ numberLabel(detailModel.capabilities.contextWindowTokens, "256000 tokens") }}
                      </div>
                    </div>
                    <div>
                      <div class="text-[11px] uppercase tracking-[0.16em] text-stone-400">最大输出长度</div>
                      <div class="mt-1 rounded-[0.45rem] bg-stone-100/75 px-3 py-2.5 text-[13px] text-stone-900">
                        {{ numberLabel(detailModel.maxOutputTokens, "64000 tokens") }}
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
  transition: background-color 160ms ease;
}

.config-select:focus {
  outline: none;
  background: rgb(255 255 255 / 0.98);
}
</style>
