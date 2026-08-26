<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  AlertTriangle,
  Brain,
  Check,
  ChevronDown,
  Image as ImageIcon,
  Info,
  Mic,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  Shield,
  Trash2,
  Type,
  Video,
  X,
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
  /** UI 不再暴露认证方式选择；保留既有值避免静默改写显式覆盖（如网关强制 Bearer）。 */
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
  /** 高级设置：模型级协议（仅限提供商已启用集合），默认 openai-completions。 */
  protocol: ProviderProtocol;
  /** 高级设置：模型级 Base URL 覆盖；空串 = 继承提供商该协议 endpoint。 */
  baseUrlOverride: string;
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
    "openai-responses": {
      enabled: false,
      baseUrl: defaultBaseUrlFor("openai-responses"),
      authType: "auto",
    },
    "openai-completions": {
      enabled: true,
      baseUrl: defaultBaseUrlFor("openai-completions"),
      authType: "auto",
    },
    "anthropic-messages": {
      enabled: false,
      baseUrl: defaultBaseUrlFor("anthropic-messages"),
      authType: "auto",
    },
  },
  apiKeyValue: "",
});

// D6：模型目录（/models 拉取）状态——列表/加载/错误复用 store 状态，选中与搜索为组件本地。
const catalogSearch = ref("");
const selectedCatalogIds = ref<string[]>([]);
const catalogPanelOpen = ref(false);
const catalogLoading = computed(() => providerStore.loadingModels);
const catalogError = computed(() => providerStore.catalogError);
const catalogIds = computed(() => providerStore.catalogModels);

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
  protocol: "openai-completions",
  baseUrlOverride: "",
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

// D1：协议徽章顺序即展示顺序；选择即代表启用该协议。
const endpointOrder: ProviderProtocol[] = ["openai-responses", "openai-completions", "anthropic-messages"];

/** 规范名 → 左列/视图徽标短标签（title 保留完整规范名）。 */
function protocolBadgeLabel(protocol: ProviderProtocol) {
  switch (protocol) {
    case "openai-responses":
      return "responses";
    case "anthropic-messages":
      return "anthropic";
    default:
      return "completions";
  }
}

// 行尾/头部弱化图标动作钮（ADR 0013 迭代二）：小尺寸、低对比，仅 hover 增强。
const ICON_ACTION_CLASS =
  "inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-[0.35rem] bg-transparent text-stone-400 transition hover:bg-[#f7e3bf] hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70 disabled:cursor-not-allowed disabled:text-stone-300 motion-reduce:transition-none";

// D9：空闲态行尾动作簇——默认隐藏，行 hover 或键盘 focus-within 时显现。
const HOVER_ACTIONS_CLASS =
  "flex shrink-0 items-center gap-0.5 opacity-0 invisible transition group-hover:opacity-100 group-hover:visible focus-within:opacity-100 focus-within:visible motion-reduce:transition-none";

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

// D5：关闭徽章时若有模型引用该协议，就地警示（不阻断）。
const modelsReferencingDisabledProtocols = computed(() => {
  if (!isEditingProvider.value || !detailProvider.value) {
    return 0;
  }
  return detailProvider.value.models.filter((model) => {
    if (!model.protocol) {
      return false;
    }
    return !providerForm.endpoints[model.protocol]?.enabled;
  }).length;
});

// D6：高级设置协议下拉仅列提供商已启用协议（含当前值兜底，防回落矛盾态）。
const modelProtocolOptions = computed<ProviderProtocol[]>(() => {
  const enabled = endpointOrder.filter(
    (protocol) =>
      detailProvider.value?.endpoints.some(
        (endpoint) => endpoint.protocol === protocol && endpoint.enabled,
      ),
  );
  if (modelForm.protocol && !enabled.includes(modelForm.protocol)) {
    return [...enabled, modelForm.protocol];
  }
  return enabled;
});

// 高级设置 Base URL 覆盖的解析结果（placeholder 展示继承目标）。
const resolvedModelBaseUrl = computed(() => {
  const override = modelForm.baseUrlOverride.trim();
  if (override) {
    return override;
  }
  const endpoint = detailProvider.value?.endpoints.find(
    (item) => item.protocol === modelForm.protocol && item.enabled,
  );
  return endpoint?.baseUrl ?? defaultBaseUrlFor(modelForm.protocol);
});

const filteredCatalogIds = computed(() => {
  const keyword = catalogSearch.value.trim().toLowerCase();
  if (!keyword) {
    return catalogIds.value;
  }
  return catalogIds.value.filter((id) => id.toLowerCase().includes(keyword));
});

const canAddSelectedCatalogModels = computed(
  () =>
    Boolean(detailProvider.value) &&
    modelCreateOpen.value &&
    selectedCatalogIds.value.length > 0 &&
    !catalogLoading.value,
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
// D8：折叠不再被编辑态锁定——收起即取消编辑（见 toggle* 与 dismiss* 家族）。
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
  "一个提供商可以同时接入多种协议；选择协议徽章即代表开启，Base URL 在高级设置中维护。";

function createEndpointRecord(
  endpoints?: ProviderProtocolEndpoint[],
): ProviderFormState["endpoints"] {
  const record: ProviderFormState["endpoints"] = {
    "openai-responses": {
      enabled: false,
      baseUrl: defaultBaseUrlFor("openai-responses"),
      authType: "auto",
    },
    "openai-completions": {
      enabled: false,
      baseUrl: defaultBaseUrlFor("openai-completions"),
      authType: "auto",
    },
    "anthropic-messages": {
      enabled: false,
      baseUrl: defaultBaseUrlFor("anthropic-messages"),
      authType: "auto",
    },
  };

  for (const endpoint of endpoints ?? []) {
    record[endpoint.protocol] = {
      enabled: endpoint.enabled,
      baseUrl: endpoint.baseUrl,
      // 保留既有非 auto 显式覆盖（如网关强制 Bearer），UI 不再提供切换入口。
      authType: endpoint.authType ?? "auto",
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
  providerForm.name = "";  providerForm.endpoints = createEndpointRecord([
    {
      protocol: "openai-completions",
      enabled: true,
      baseUrl: defaultBaseUrlFor("openai-completions"),
      authType: "auto",
    },
    {
      protocol: "openai-responses",
      enabled: false,
      baseUrl: defaultBaseUrlFor("openai-responses"),
      authType: "auto",
    },
    {
      protocol: "anthropic-messages",
      enabled: false,
      baseUrl: defaultBaseUrlFor("anthropic-messages"),
      authType: "auto",
    },
  ]);
  providerForm.apiKeyValue = "";
  providerFormContextId.value = null;
}

function fillProviderForm(provider: ProviderConfig) {
  providerForm.name = provider.name;
  providerForm.endpoints = createEndpointRecord(provider.endpoints);
  providerForm.apiKeyValue = provider.apiKeyValue;
  // 记录表单密钥归属，目录拉取仅在该提供商上下文回传未保存 key（防跨提供商误用）。
  providerFormContextId.value = provider.id;
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
  // 高级设置默认值：协议默认 openai-completions；Base URL 覆盖清空（=继承）。
  const enabledProtocols = endpointOrder.filter((protocol) =>
    detailProvider.value?.endpoints.some(
      (endpoint) => endpoint.protocol === protocol && endpoint.enabled,
    ),
  );
  modelForm.protocol = enabledProtocols.includes("openai-completions")
    ? "openai-completions"
    : enabledProtocols[0] ?? "openai-completions";
  modelForm.baseUrlOverride = "";
  resetCatalogState();
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
  modelForm.protocol = normalizeProtocolValue(model.protocol ?? modelForm.protocol);
  modelForm.baseUrlOverride = model.baseUrl?.trim() ?? "";
  resetCatalogState();
}

function normalizeProtocolValue(value: unknown): ProviderProtocol {
  switch (value) {
    case "openai-responses":
    case "openai-completions":
    case "anthropic-messages":
      return value;
    default:
      return "openai-completions";
  }
}

// D5：徽章选择即启用；至少保留一个启用协议（最后一个不可关闭）。
const providerAdvancedOpen = ref(false);
const modelAdvancedOpen = ref(false);
// 表单密钥的归属提供商（fillProviderForm 时登记）：目录拉取仅在该上下文回传未保存 key。
const providerFormContextId = ref<string | null>(null);

function toggleProviderProtocol(protocol: ProviderProtocol) {
  const state = providerForm.endpoints[protocol];
  if (!state.enabled) {
    state.enabled = true;
    return;
  }
  if (providerEnabledProtocols.value.length <= 1) {
    return;
  }
  state.enabled = false;
}

function resetCatalogState() {
  catalogSearch.value = "";
  selectedCatalogIds.value = [];
  catalogPanelOpen.value = false;
  providerStore.clearModelCatalog();
}

// D6（迭代五）：刷新图标按钮拉取 /models 目录；成功后自动展开下拉面板，
// 失败保持收起——失败语义经选择器旁的 info 图标 + tooltip 呈现。
async function fetchModelCatalog() {
  if (!detailProvider.value || catalogLoading.value) {
    return;
  }

  selectedCatalogIds.value = [];
  // D6（code-review B/P1-1）：表单中已输入但未保存的密钥仅在归属同一提供商时回传，
  // 避免把 A 的草稿密钥发往 B 的 base_url。
  const unsavedApiKey =
    providerFormContextId.value === detailProvider.value.id
      ? providerForm.apiKeyValue.trim()
      : "";
  const models = await providerStore.fetchModelCatalog({
    providerId: detailProvider.value.id,
    protocol: modelForm.protocol,
    baseUrl: resolvedModelBaseUrl.value,
    apiKey: unsavedApiKey || undefined,
  });
  catalogPanelOpen.value = models.length > 0 && !providerStore.catalogError;
}

function toggleCatalogPanel() {
  if (catalogIds.value.length === 0) {
    return;
  }
  catalogPanelOpen.value = !catalogPanelOpen.value;
}

function toggleCatalogSelection(modelId: string) {
  const index = selectedCatalogIds.value.indexOf(modelId);
  if (index >= 0) {
    selectedCatalogIds.value.splice(index, 1);
    return;
  }
  selectedCatalogIds.value.push(modelId);
}

// D6：批量添加走专用 action，预去重并返回 {added, skipped}，不复用 this.error 通道。
async function addSelectedCatalogModels() {
  const provider = detailProvider.value;
  if (!provider || selectedCatalogIds.value.length === 0) {
    return;
  }

  const result = providerStore.addModelsFromCatalog(provider.id, [...selectedCatalogIds.value], {
    protocol: modelForm.protocol,
    baseUrl: modelForm.baseUrlOverride.trim() || null,
  });

  if (result.added > 0) {
    await providerStore.saveRegistry();
  }

  if (!providerStore.error) {
    const suffix =
      result.skipped > 0
        ? `，跳过 ${result.skipped} 个已存在（同 ID 按别名折叠）`
        : "";
    providerStore.notice = `已添加 ${result.added} 个模型${suffix}。`;
    selectedCatalogIds.value = [];
    if (result.added > 0 && result.lastAddedModelId) {
      beginViewModel(provider.id, result.lastAddedModelId);
    }
  }
}

// D8：收起即取消——任何使承载活动编辑/创建的区（或展开行）收起的翻转，
// 都先取消该区编辑再折叠，含手风琴从另一区触发的连带收起。create 态收起 = 放弃并复位。

function dismissModelEditing() {
  if (modelCreateOpen.value) {
    modelCreateOpen.value = false;
    resetModelForm();
    editorState.entity = "provider";
    editorState.mode = "view";
    editorState.modelId = null;
    return;
  }
  const pid = editorState.providerId;
  const mid = editorState.modelId;
  if (pid && mid && findModel(pid, mid)) {
    beginViewModel(pid, mid);
    return;
  }
  if (pid) {
    beginViewProvider(pid);
  }
}

function dismissProviderEditing() {
  resetProviderForm();
  if (currentProvider.value) {
    beginViewProvider(currentProvider.value.id);
    return;
  }
  // 无任何提供商：保留 create 态但允许整体收起；重新展开为空白新建表单。
}

function toggleProviderSection() {
  if (providerSectionOpen.value) {
    if (isEditingProvider.value) {
      dismissProviderEditing();
    }
    providerSectionOpen.value = false;
    return;
  }
  // 展开提供商区 → 手风琴连带收起模型区：先取消模型区活动编辑。
  if (modelSectionOpen.value && (isEditingModel.value || modelCreateOpen.value)) {
    dismissModelEditing();
  }
  providerSectionOpen.value = true;
  modelSectionOpen.value = false;
}

function toggleModelSection() {
  if (modelSectionOpen.value) {
    if (isEditingModel.value || modelCreateOpen.value) {
      dismissModelEditing();
    }
    modelSectionOpen.value = false;
    return;
  }
  // 展开模型列表区 → 手风琴连带收起提供商区：先取消提供商区活动编辑。
  if (providerSectionOpen.value && isEditingProvider.value) {
    dismissProviderEditing();
  }
  modelSectionOpen.value = true;
  providerSectionOpen.value = false;
}

// D8：模型行点击——编辑中再次点击该行 → 先取消编辑再收起回落提供商视图；
// 点击其他行先取消当前编辑/创建卡再展开目标；空闲态行为不变。
function toggleModelRow(providerId: string, modelId: string) {
  if (isEditingProvider.value) {
    return;
  }
  if (expandedModelId.value === modelId) {
    if ((isEditingModel.value && editorState.modelId === modelId) || modelCreateOpen.value) {
      dismissModelEditing();
    }
    beginViewProvider(providerId);
    return;
  }
  if (isEditingModel.value || modelCreateOpen.value) {
    dismissModelEditing();
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
    // P1-2 防护：create 首次落地后切换为 edit，失败重试复用同一 providerId，
    // 不再二次 addProvider 产生孤儿半配置提供商。
    if (providerId) {
      editorState.mode = "edit";
      editorState.providerId = providerId;
    }
  }
  if (!providerId) {
    return;
  }

  const endpoints = buildProviderEndpoints();
  // D5（code-review A/P2-1）：既有主协议仍启用时保持其首位，避免勾选新徽章
  // （如 openai-responses）静默翻转提供商主协议/base_url 与新模型的默认协议。
  const enabledProtocols = endpoints.filter((item) => item.enabled).map((item) => item.protocol);
  const previousPrimary = findProvider(providerId)?.protocol;
  const supportedProtocols =
    previousPrimary && enabledProtocols.includes(previousPrimary)
      ? [
          previousPrimary,
          ...enabledProtocols.filter((protocol) => protocol !== previousPrimary),
        ]
      : enabledProtocols;
  const primaryProtocol = supportedProtocols[0] ?? "openai-completions";
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
        // D6 高级设置：模型级协议（默认 openai-completions）+ Base URL 覆盖。
        protocol: modelForm.protocol,
        baseUrl: modelForm.baseUrlOverride.trim() || null,
      },
      resolveCapabilityDeclaration(
        modelForm.protocol,
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

function providerApiKeySummary(provider: ProviderConfig) {
  if (provider.apiKeyValue?.trim()) {
    return "已填写待保存的新密钥";
  }

  return provider.apiKeyPresent ? "已有已保存密钥" : "未配置";
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
            <!-- 迭代四：协议与模型数徽标同行尾部显示，不再换行堆叠；D1 徽标直显规范名短标签 -->
            <span class="flex shrink-0 items-center gap-1">
              <span
                v-for="protocol in provider.supportedProtocols"
                :key="protocol"
                class="rounded-[0.2rem] bg-white/40 px-1.5 py-[1px] font-mono text-[9px] leading-[1.4] text-stone-400/80"
                :title="protocol"
              >
                {{ protocolBadgeLabel(protocol) }}
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
              <!-- D8：整行为折叠 trigger，编辑态不再锁定——收起即取消编辑；
                   D9：空闲态动作簇 hover/focus-within 显隐（容器需 group 供 group-hover 生效）。 -->
              <div
                class="group -mx-1 flex min-h-[1.75rem] cursor-pointer items-center justify-between gap-2 rounded-[0.35rem] px-1"
                :title="isEditingProvider ? '收起将放弃未保存的修改' : undefined"
                data-testid="provider-detail-header"
                @click="toggleProviderSection()"
              >
                <button
                  type="button"
                  class="flex min-w-0 cursor-pointer flex-1 items-center gap-1 bg-transparent text-left outline-none"
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

                <!-- 空闲动作簇：hover 显隐；编辑动作簇：常显（保存不可见即死状态）。 -->
                <div v-if="!isEditingProvider" :class="HOVER_ACTIONS_CLASS" @click.stop>
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
                </div>
                <div v-else class="flex shrink-0 items-center gap-0.5" @click.stop>
                  <Button size="sm" variant="ghost" data-testid="provider-edit-cancel" @click="cancelEditing()">取消</Button>
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
                <div class="rounded-[0.45rem] bg-white/72 px-3.5 py-3">
                  <!-- D5（迭代五·排版修订）：统一「标签列 | 控件列」网格——左标签定宽右对齐，
                       控件列同起点等宽，既不拥挤也不散排；与全页"字段名+值同行"语言一致。 -->
                  <div class="grid grid-cols-[minmax(56px,auto)_minmax(0,1fr)] items-center gap-x-4 gap-y-2.5">
                    <label class="contents">
                      <span class="text-[11px] text-stone-500">名称</span>
                      <Input :model-value="providerForm.name" placeholder="例如：OpenRouter" @update:model-value="providerForm.name = $event" />
                    </label>

                    <div class="contents">
                      <span class="self-center text-[11px] text-stone-500">协议</span>
                      <div class="flex min-w-0 flex-wrap items-center gap-1.5" role="group" aria-label="协议">
                        <button
                          v-for="protocol in endpointOrder"
                          :key="protocol"
                          type="button"
                          class="cursor-pointer rounded-full px-2.5 py-1 font-mono text-[11px] leading-[1.4] transition"
                          :class="
                            providerForm.endpoints[protocol].enabled
                              ? 'bg-stone-900 text-white'
                              : 'bg-white/85 text-stone-500 ring-1 ring-stone-200/80 hover:bg-[#f7e3bf] hover:text-stone-900'
                          "
                          :aria-pressed="providerForm.endpoints[protocol].enabled"
                          :data-testid="`provider-protocol-badge-${protocol}`"
                          :title="providerForm.endpoints[protocol].enabled ? '已启用，点击关闭' : '点击启用'"
                          @click="toggleProviderProtocol(protocol)"
                        >
                          {{ protocol }}
                        </button>
                        <InfoTip text="选择即代表开启该协议；各协议 Base URL 在下方高级设置中维护，认证方式由协议自动推导。" />
                      </div>
                    </div>

                    <label class="contents">
                      <span class="text-[11px] text-stone-500">密钥</span>
                      <Input
                        :model-value="providerForm.apiKeyValue"
                        type="password"
                        placeholder="API Key，输入后保存即可"
                        @update:model-value="providerForm.apiKeyValue = $event"
                      />
                    </label>

                    <!-- 关徽警示：对齐到控件列 -->
                    <p
                      v-if="modelsReferencingDisabledProtocols > 0"
                      class="col-start-2 rounded-[0.35rem] bg-amber-50/90 px-2.5 py-1.5 text-[11px] leading-4 text-amber-900"
                      data-testid="provider-badge-off-warning"
                    >
                      {{ modelsReferencingDisabledProtocols }} 个模型正在使用被关闭的协议，保存后将回落主协议。
                    </p>
                  </div>

                  <!-- 高级设置独立分区：分隔线 + 折叠头 + 同构网格的 Base URL 行 -->
                  <div class="mt-2 border-t border-stone-200/60 pt-2">
                    <button
                      type="button"
                      class="inline-flex cursor-pointer items-center gap-1 rounded text-[11px] text-stone-500 transition hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
                      data-testid="provider-advanced-toggle"
                      :aria-expanded="providerAdvancedOpen"
                      @click="providerAdvancedOpen = !providerAdvancedOpen"
                    >
                      <ChevronDown
                        class="h-3 w-3 shrink-0 transition-transform duration-200 motion-reduce:transition-none"
                        :class="providerAdvancedOpen ? 'rotate-180' : ''"
                      />
                      高级设置 · 各协议 Base URL
                    </button>
                    <div
                      v-show="providerAdvancedOpen"
                      class="mt-2 grid grid-cols-[minmax(56px,auto)_minmax(0,1fr)] items-center gap-x-4 gap-y-2"
                      data-testid="provider-advanced-body"
                    >
                      <label
                        v-for="protocol in providerEnabledProtocols"
                        :key="protocol"
                        class="contents"
                      >
                        <span class="truncate font-mono text-[10px] leading-tight text-stone-400" :title="protocol">{{ protocolBadgeLabel(protocol) }}</span>
                        <Input
                          :model-value="providerForm.endpoints[protocol].baseUrl"
                          :placeholder="defaultBaseUrlFor(protocol)"
                          :data-testid="`provider-baseurl-${protocol}`"
                          @update:model-value="providerForm.endpoints[protocol].baseUrl = $event"
                        />
                      </label>
                    </div>
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
                  <div class="text-sm font-medium text-stone-900">协议</div>
                  <div class="mt-1.5 space-y-1">
                    <div
                      v-for="endpoint in detailProvider.endpoints.filter((item) => item.enabled)"
                      :key="endpoint.protocol"
                      class="flex min-w-0 items-baseline justify-between gap-3 rounded-[0.35rem] bg-stone-100/75 px-2.5 py-1.5"
                    >
                      <span class="shrink-0 font-mono text-[11px] text-stone-900" :title="endpoint.protocol">{{ endpoint.protocol }}</span>
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
                class="group -mx-1 flex min-h-[1.75rem] cursor-pointer items-center justify-between gap-2 rounded-[0.35rem] px-1"
                :title="isEditingModel || modelCreateOpen ? '收起将放弃未保存的修改' : undefined"
                data-testid="model-list-header"
                @click="toggleModelSection()"
              >
                <button
                  type="button"
                  class="flex min-w-0 cursor-pointer flex-1 items-center gap-1 bg-transparent text-left outline-none"
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

                <div v-if="modelActionsIdle" :class="HOVER_ACTIONS_CLASS" @click.stop>
                  <Tooltip v-if="detailProvider" text="新增模型" side="top">
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

                      <!-- D6（迭代五）：模型 ID 选择器——刷新图标在选择器右端，info 图标在外侧；
                           成功自动展开目录下拉，失败经 info 图标 + tooltip 呈现。 -->
                      <div class="space-y-1 text-[11px] text-stone-500">
                        <div class="flex items-center gap-2">
                          <span class="shrink-0">模型 ID</span>
                          <div class="relative min-w-0 flex-1">
                            <Input
                              :model-value="modelForm.model"
                              placeholder="手动填入，或点刷新从目录选择"
                              class="pr-16"
                              data-testid="model-id-input"
                              @update:model-value="modelForm.model = $event"
                            />
                            <div class="absolute right-1.5 top-1/2 flex -translate-y-1/2 items-center gap-0.5">
                              <button
                                type="button"
                                class="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7e3bf] hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70 disabled:cursor-not-allowed disabled:text-stone-300"
                                :disabled="catalogLoading && catalogIds.length === 0"
                                aria-label="从 /models 刷新模型列表"
                                data-testid="model-catalog-fetch"
                                @click.stop="fetchModelCatalog()"
                              >
                                <RefreshCw class="h-3.5 w-3.5" :class="catalogLoading ? 'animate-spin' : ''" />
                              </button>
                              <button
                                v-if="catalogIds.length > 0"
                                type="button"
                                class="inline-flex h-7 w-5 cursor-pointer items-center justify-center rounded text-stone-400 transition hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
                                :aria-expanded="catalogPanelOpen"
                                aria-label="展开或收起模型目录"
                                data-testid="model-catalog-toggle"
                                @click.stop="toggleCatalogPanel()"
                              >
                                <ChevronDown
                                  class="h-3.5 w-3.5 transition-transform duration-200 motion-reduce:transition-none"
                                  :class="catalogPanelOpen ? 'rotate-180' : ''"
                                />
                              </button>
                            </div>
                          </div>

                          <!-- info / 失败语义图标：选择器外侧 -->
                          <Tooltip
                            :text="
                              catalogError
                                ? catalogError
                                : '点击右端刷新按钮，从该协议的 /models 接口获取可选模型 ID；在下拉列表中勾选后可批量添加'
                            "
                            side="top"
                          >
                            <span
                              class="inline-flex shrink-0 cursor-help items-center justify-center"
                              :class="catalogError ? 'text-rose-500' : 'text-stone-400'"
                              :data-testid="catalogError ? 'model-catalog-info-error' : 'model-catalog-info'"
                            >
                              <AlertTriangle v-if="catalogError" class="h-4 w-4" />
                              <Info v-else class="h-4 w-4" />
                            </span>
                          </Tooltip>
                        </div>

                        <div v-if="selectedCatalogIds.length > 0" class="flex flex-wrap gap-1 pt-0.5">
                          <span
                            v-for="id in selectedCatalogIds"
                            :key="id"
                            class="inline-flex max-w-full items-center gap-1 rounded-full bg-stone-900/85 py-[2px] pl-2 pr-1 font-mono text-[10px] leading-[1.4] text-stone-50"
                            :data-testid="`model-catalog-chip-${id}`"
                          >
                            <span class="truncate">{{ id }}</span>
                            <button type="button" class="cursor-pointer rounded-full p-0.5 hover:bg-white/20" :aria-label="`移除 ${id}`" @click="toggleCatalogSelection(id)">
                              <X class="h-3 w-3" />
                            </button>
                          </span>
                        </div>

                        <div
                          v-if="catalogIds.length > 0 && catalogPanelOpen"
                          class="rounded-[0.35rem] bg-white p-2 ring-1 ring-stone-200/70"
                          data-testid="model-catalog-panel"
                        >
                          <div class="flex items-center gap-2 pb-1.5">
                            <Search class="h-3.5 w-3.5 shrink-0 text-stone-400" />
                            <input
                              :value="catalogSearch"
                              placeholder="搜索模型..."
                              class="h-7 w-full min-w-0 rounded-[0.3rem] bg-stone-100/80 px-2 text-[12px] text-stone-900 outline-none transition focus:bg-white"
                              data-testid="model-catalog-search"
                              @input="catalogSearch = ($event.target as HTMLInputElement).value"
                            />
                            <span class="shrink-0 text-[10px] text-stone-400">{{ selectedCatalogIds.length }}/{{ catalogIds.length }}</span>
                          </div>
                          <div class="max-h-40 space-y-0.5 overflow-y-auto">
                            <label
                              v-for="id in filteredCatalogIds"
                              :key="id"
                              class="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 font-mono text-[11px] text-stone-800 transition hover:bg-[#f7e3bf]/60"
                            >
                              <input
                                type="checkbox"
                                class="accent-stone-900"
                                :checked="selectedCatalogIds.includes(id)"
                                :data-testid="`model-catalog-option-${id}`"
                                @change="toggleCatalogSelection(id)"
                              />
                              <span class="truncate">{{ id }}</span>
                            </label>
                          </div>
                          <Button
                            v-if="canAddSelectedCatalogModels"
                            size="sm"
                            variant="secondary"
                            class="mt-1.5 w-full justify-center"
                            data-testid="model-catalog-add-selected"
                            @click="addSelectedCatalogModels()"
                          >
                            <Plus class="mr-1 h-3.5 w-3.5" />
                            添加选中的 {{ selectedCatalogIds.length }} 个模型
                          </Button>
                        </div>
                      </div>
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
                    <!-- D6：高级设置——协议（默认 openai-completions）与 Base URL 覆盖。 -->
                    <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                      <button
                        type="button"
                        class="inline-flex cursor-pointer items-center gap-1 text-[13px] font-medium text-stone-900 transition hover:text-stone-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
                        data-testid="model-advanced-toggle-create"
                        :aria-expanded="modelAdvancedOpen"
                        @click="modelAdvancedOpen = !modelAdvancedOpen"
                      >
                        <ChevronDown
                          class="h-3.5 w-3.5 shrink-0 text-stone-400 transition-transform duration-200 motion-reduce:transition-none"
                          :class="modelAdvancedOpen ? 'rotate-180' : ''"
                        />
                        高级设置
                        <InfoTip text="模型可改用提供商已开启的其他协议，并可覆盖 Base URL；默认 openai-completions、继承提供商地址。" />
                      </button>
                      <div
                        v-show="modelAdvancedOpen"
                        class="mt-2 grid gap-2.5 xl:grid-cols-2"
                        data-testid="model-advanced-body-create"
                      >
                        <label class="space-y-1 text-[11px] text-stone-500">
                          <span>协议</span>
                          <select
                            v-model="modelForm.protocol"
                            class="config-select cursor-pointer"
                            data-testid="model-advanced-protocol-create"
                          >
                            <option v-for="protocol in modelProtocolOptions" :key="protocol" :value="protocol">{{ protocol }}</option>
                          </select>
                        </label>
                        <label class="space-y-1 text-[11px] text-stone-500">
                          <span>Base URL 覆盖</span>
                          <Input
                            :model-value="modelForm.baseUrlOverride"
                            :placeholder="resolvedModelBaseUrl"
                            data-testid="model-advanced-baseurl-create"
                            @update:model-value="modelForm.baseUrlOverride = $event"
                          />
                        </label>
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
                    <!-- D8/D9：整行为折叠 trigger（编辑中收起即取消）；行容器 group，
                         行尾 编辑/删除 悬停显隐，chevron 恒显为纯指示器。 -->
                    <div
                      class="group flex cursor-pointer items-center justify-between gap-2 rounded-[0.35rem] px-2 py-1.5 transition-colors hover:bg-white/74 motion-reduce:transition-none"
                      :data-testid="`model-list-item-${model.id}`"
                      @click="toggleModelRow(detailProvider!.id, model.id)"
                    >
                      <button
                        type="button"
                        class="flex min-w-0 cursor-pointer flex-1 items-center rounded-[0.35rem] px-1 py-0.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
                        :aria-expanded="expandedModelId === model.id"
                        :aria-controls="`model-detail-${model.id}`"
                      >
                        <span class="min-w-0">
                          <span class="block truncate text-[12px] font-medium text-stone-800">
                            {{ model.name || "未命名模型" }}
                          </span>
                          <span class="mt-0.5 block truncate font-mono text-[11px] text-stone-500">
                            {{ model.protocol || detailProvider?.protocol }} · {{ model.model || "未填写模型 ID" }}
                          </span>
                        </span>
                      </button>

                      <div :class="HOVER_ACTIONS_CLASS" @click.stop>
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

                  <!-- D6（迭代五）：编辑态同款选择器——刷新图标在输入框右端、info 在外侧；
                       目录下拉单击即填充（单选，无批量）。 -->
                  <div class="space-y-1 text-[11px] text-stone-500">
                    <div class="flex items-center gap-2">
                      <span class="shrink-0">模型 ID</span>
                      <div class="relative min-w-0 flex-1">
                        <Input
                          :model-value="modelForm.model"
                          placeholder="例如：claude-sonnet-4-20250514"
                          class="pr-16"
                          @update:model-value="modelForm.model = $event"
                        />
                        <div class="absolute right-1.5 top-1/2 flex -translate-y-1/2 items-center gap-0.5">
                          <button
                            type="button"
                            class="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-[#f7e3bf] hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70 disabled:cursor-not-allowed disabled:text-stone-300"
                            :disabled="catalogLoading && catalogIds.length === 0"
                            aria-label="从 /models 刷新模型列表"
                            data-testid="model-catalog-fetch-edit"
                            @click.stop="fetchModelCatalog()"
                          >
                            <RefreshCw class="h-3.5 w-3.5" :class="catalogLoading ? 'animate-spin' : ''" />
                          </button>
                          <button
                            v-if="filteredCatalogIds.length > 0"
                            type="button"
                            class="inline-flex h-7 w-5 cursor-pointer items-center justify-center rounded text-stone-400 transition hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
                            :aria-expanded="catalogPanelOpen"
                            aria-label="展开或收起模型目录"
                            data-testid="model-catalog-toggle-edit"
                            @click.stop="toggleCatalogPanel()"
                          >
                            <ChevronDown
                              class="h-3.5 w-3.5 transition-transform duration-200 motion-reduce:transition-none"
                              :class="catalogPanelOpen ? 'rotate-180' : ''"
                            />
                          </button>
                        </div>
                      </div>

                      <Tooltip
                        :text="
                          catalogError
                            ? catalogError
                            : '点击右端刷新按钮，从该协议的 /models 接口获取可选模型 ID；在下拉列表中单击即填充'
                        "
                        side="top"
                      >
                        <span
                          class="inline-flex shrink-0 cursor-help items-center justify-center"
                          :class="catalogError ? 'text-rose-500' : 'text-stone-400'"
                          :data-testid="catalogError ? 'model-catalog-info-error-edit' : 'model-catalog-info-edit'"
                        >
                          <AlertTriangle v-if="catalogError" class="h-4 w-4" />
                          <Info v-else class="h-4 w-4" />
                        </span>
                      </Tooltip>
                    </div>

                    <div
                      v-if="filteredCatalogIds.length > 0 && catalogPanelOpen"
                      class="max-h-24 space-y-0.5 overflow-y-auto rounded-[0.35rem] bg-white p-1.5 ring-1 ring-stone-200/70"
                      data-testid="model-catalog-panel-edit"
                    >
                      <button
                        v-for="id in filteredCatalogIds"
                        :key="id"
                        type="button"
                        class="block w-full cursor-pointer truncate rounded px-2 py-1 text-left font-mono text-[11px] text-stone-700 transition hover:bg-[#f7e3bf]/60 hover:text-stone-900"
                        :data-testid="`model-catalog-suggest-${id}`"
                        @click="modelForm.model = id; catalogPanelOpen = false"
                      >
                        {{ id }}
                      </button>
                    </div>
                  </div>
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

                <!-- D6：高级设置——编辑态同款（协议/Base URL 覆盖），与创建卡同步维护。 -->
                <section class="rounded-[0.45rem] bg-white/72 px-3.5 py-2">
                  <button
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1 text-[13px] font-medium text-stone-900 transition hover:text-stone-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
                    data-testid="model-advanced-toggle-edit"
                    :aria-expanded="modelAdvancedOpen"
                    @click="modelAdvancedOpen = !modelAdvancedOpen"
                  >
                    <ChevronDown
                      class="h-3.5 w-3.5 shrink-0 text-stone-400 transition-transform duration-200 motion-reduce:transition-none"
                      :class="modelAdvancedOpen ? 'rotate-180' : ''"
                    />
                    高级设置
                    <InfoTip text="模型可改用提供商已开启的其他协议，并可覆盖 Base URL。" />
                  </button>
                  <div
                    v-show="modelAdvancedOpen"
                    class="mt-2 grid gap-2.5 xl:grid-cols-2"
                    data-testid="model-advanced-body-edit"
                  >
                    <label class="space-y-1 text-[11px] text-stone-500">
                      <span>协议</span>
                      <select
                        v-model="modelForm.protocol"
                        class="config-select cursor-pointer"
                        data-testid="model-advanced-protocol-edit"
                      >
                        <option v-for="protocol in modelProtocolOptions" :key="protocol" :value="protocol">{{ protocol }}</option>
                      </select>
                    </label>
                    <label class="space-y-1 text-[11px] text-stone-500">
                      <span>Base URL 覆盖</span>
                      <Input
                        :model-value="modelForm.baseUrlOverride"
                        :placeholder="resolvedModelBaseUrl"
                        data-testid="model-advanced-baseurl-edit"
                        @update:model-value="modelForm.baseUrlOverride = $event"
                      />
                    </label>
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

/* D7：输入面统一与卡片底色区分的白底（含 select/textarea 与目录搜索框）。 */
.config-form :deep(input:focus-visible),
.model-catalog-search:focus,
.catalog-search-input:focus {
  background: rgb(255 255 255 / 0.98);
  outline: none;
}

.config-form :deep(select),
.config-form :deep(textarea) {
  border-radius: 0.45rem;
  border: none;
  background: rgb(255 255 255 / 0.82);
  padding-inline: 0.75rem;
  font-size: 0.875rem;
  color: rgb(28 25 23);
  outline: none;
  transition: background-color 160ms ease;
}

.config-form :deep(select:focus),
.config-form :deep(textarea:focus) {
  background: rgb(255 255 255 / 0.98);
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
