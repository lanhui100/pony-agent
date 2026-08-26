import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type {
  ProviderAuthType,
  ProviderCapabilityPresetId,
  ProviderConfig,
  ProviderModelCapabilities,
  ProviderModelCapabilityDeclaration,
  ProviderModelConfig,
  ProviderModelIdentity,
  ProviderModelUserPolicy,
  ProviderProtocol,
  ProviderProtocolEndpoint,
  ProviderReasoningEffort,
  ProviderRegistry,
} from "@/types/provider";

const DEFAULT_CONTEXT_WINDOW_TOKENS = 256000;
const DEFAULT_MAX_OUTPUT_TOKENS = 64000;

// 协议家族（能力目录匹配用）：openai 族 = completions + responses 两值。
type ProviderProtocolFamily = "openai" | "anthropic";

export function protocolFamilyOf(protocol: ProviderProtocol): ProviderProtocolFamily {
  return protocol === "anthropic-messages" ? "anthropic" : "openai";
}

export function isAnthropicProtocol(protocol: ProviderProtocol): boolean {
  return protocolFamilyOf(protocol) === "anthropic";
}

// D1：旧值规范化单点。旧 providers.json/旧 trace 的 openai/anthropic 映射到规范名，
// 未知值一律回落 openai-completions，杜绝未知名继续在内存态扩散。
export function normalizeLegacyProtocol(value: unknown): ProviderProtocol {
  switch (value) {
    case "openai-responses":
    case "openai-completions":
    case "anthropic-messages":
      return value;
    case "anthropic":
      return "anthropic-messages";
    default:
      // 含旧值 "openai" 与一切未知值。
      return "openai-completions";
  }
}

type CapabilityFactCatalogEntry = {
  /** 家族标签（非 wire 值）：openai 族两值均可命中 openai 条目。 */
  protocol?: ProviderProtocolFamily;
  patterns: string[];
  preset: Exclude<ProviderCapabilityPresetId, "auto" | "custom">;
};

const CAPABILITY_CATALOG: CapabilityFactCatalogEntry[] = [
  {
    protocol: "anthropic",
    patterns: ["claude-3-7", "claude-sonnet-4", "claude-opus-4"],
    preset: "anthropic-thinking",
  },
  {
    protocol: "openai",
    patterns: ["gpt-5", "gpt-5.4", "gpt-5.5", "o1", "o3", "reason"],
    preset: "open-ai-reasoning",
  },
  {
    protocol: "openai",
    patterns: ["gpt-4.1", "vision"],
    preset: "open-ai-chat",
  },
  {
    protocol: "openai",
    patterns: ["deepseek-reasoner", "deepseek-r1", "deepseek-v4-pro"],
    preset: "deepseek-reasoner",
  },
  {
    protocol: "openai",
    patterns: ["deepseek-chat", "deepseek-v4-flash"],
    preset: "deepseek-chat",
  },
];

export const ALL_PROVIDER_PROTOCOLS: ProviderProtocol[] = [
  "openai-responses",
  "openai-completions",
  "anthropic-messages",
];

export function defaultBaseUrlFor(protocol: ProviderProtocol) {
  return isAnthropicProtocol(protocol)
    ? "https://api.anthropic.com/v1"
    : "https://api.openai.com/v1";
}

// D1：UI 不再暴露认证方式选择；新建 endpoint 一律写 auto，由后端按家族解析；
// 既有非 auto 值在 normalizeEndpoints 中保留、不被静默改写。
function defaultAuthTypeFor(_protocol: ProviderProtocol): ProviderAuthType {
  return "auto";
}

function createDefaultEndpoint(
  protocol: ProviderProtocol,
  overrides: Partial<ProviderProtocolEndpoint> = {},
): ProviderProtocolEndpoint {
  return {
    protocol,
    enabled: false,
    baseUrl: defaultBaseUrlFor(protocol),
    authType: defaultAuthTypeFor(protocol),
    ...overrides,
  };
}

function getDefaultEndpoints(): ProviderProtocolEndpoint[] {
  return [
    createDefaultEndpoint("openai-completions", { enabled: true, authType: "auto" }),
    createDefaultEndpoint("openai-responses"),
    createDefaultEndpoint("anthropic-messages"),
  ];
}

export function createDefaultCapabilities(): ProviderModelCapabilities {
  return {
    contextWindowTokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
    supportsTools: true,
    supportsStreaming: true,
    supportsReasoning: false,
    supportsImageInput: false,
    supportsVideoInput: false,
    supportsAudioInput: false,
    supportsTextOutput: true,
    supportsImageOutput: false,
    supportsVideoOutput: false,
    supportsAudioOutput: false,
  };
}

export function createDefaultModelUserPolicy(): ProviderModelUserPolicy {
  return {
    temperature: 0,
    maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
    reasoningEffort: null,
    reasoningBudgetTokens: null,
  };
}

function getSafeCapabilities(
  capabilities: Partial<ProviderModelCapabilities> | null | undefined,
): ProviderModelCapabilities {
  return {
    ...createDefaultCapabilities(),
    ...(capabilities ?? {}),
  };
}

function normalizeCapabilityPresetId(
  preset: ProviderCapabilityPresetId | string | null | undefined,
  protocol: ProviderProtocol,
  modelIdValue: string,
): ProviderCapabilityPresetId {
  switch (preset) {
    case "auto":
    case "open-ai-chat":
    case "open-ai-reasoning":
    case "anthropic-thinking":
    case "deepseek-chat":
    case "deepseek-reasoner":
    case "custom":
      return preset;
    case "openai-chat":
      return "open-ai-chat";
    case "openai-reasoning":
      return "open-ai-reasoning";
    default:
      return inferCapabilityPreset(protocol, modelIdValue);
  }
}

function inferCapabilityPreset(
  protocol: ProviderProtocol,
  modelIdValue: string,
): Exclude<ProviderCapabilityPresetId, "custom"> {
  const lower = modelIdValue.toLowerCase();

  // 家族匹配：openai-completions 与 openai-responses 均可命中 "openai" 条目。
  const family = protocolFamilyOf(protocol);
  const matched = CAPABILITY_CATALOG.find(
    (entry) =>
      (!entry.protocol || entry.protocol === family) &&
      entry.patterns.some((pattern) => lower.includes(pattern)),
  );
  if (matched) {
    return matched.preset;
  }

  if (isAnthropicProtocol(protocol)) {
    return "anthropic-thinking";
  }

  return "auto";
}

function createCapabilities(
  overrides: Partial<ProviderModelCapabilities>,
): ProviderModelCapabilities {
  return {
    ...createDefaultCapabilities(),
    ...overrides,
  };
}

function capabilitiesForPreset(
  preset: Exclude<ProviderCapabilityPresetId, "custom">,
  protocol: ProviderProtocol,
  modelIdValue: string,
): ProviderModelCapabilities {
  const inferredPreset =
    preset === "auto" ? inferCapabilityPreset(protocol, modelIdValue) : preset;
  const lower = modelIdValue.toLowerCase();

  switch (inferredPreset) {
    case "open-ai-chat":
      return createCapabilities({
        supportsReasoning: false,
        supportsImageInput: true,
        supportsTextOutput: true,
      });
    case "open-ai-reasoning":
      return createCapabilities({
        supportsReasoning: true,
        supportsTextOutput: true,
      });
    case "anthropic-thinking":
      return createCapabilities({
        supportsReasoning: true,
        supportsImageInput: true,
        supportsTextOutput: true,
      });
    case "deepseek-chat":
      return createCapabilities({
        supportsReasoning: false,
        supportsTextOutput: true,
      });
    case "deepseek-reasoner":
      return createCapabilities({
        supportsReasoning: true,
        supportsTextOutput: true,
      });
    case "auto":
      return createCapabilities({
        supportsReasoning:
          lower.includes("gpt-5") ||
          lower.includes("o1") ||
          lower.includes("o3") ||
          lower.includes("reason") ||
          lower.includes("claude-3-7") ||
          lower.includes("deepseek-r1") ||
          lower.includes("deepseek-reasoner") ||
          lower.includes("deepseek-v4-pro"),
        supportsImageInput:
          lower.includes("gpt-4.1") ||
          lower.includes("claude") ||
          lower.includes("vision"),
      });
    default:
      return capabilitiesForPreset("auto", protocol, modelIdValue);
  }
}

function normalizeNullablePositiveInteger(
  value: number | null | undefined,
): number | null {
  if (!Number.isFinite(value) || !value || value <= 0) {
    return null;
  }

  return Math.trunc(value);
}

function createId(prefix: string) {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}-${crypto.randomUUID()}`;
  }

  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
}

function deriveEnvVarName(providerName: string) {
  const envName = providerName
    .trim()
    .replace(/[^a-zA-Z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .toUpperCase();

  return envName ? `${envName}_API_KEY` : "CUSTOM_PROVIDER_API_KEY";
}

function normalizeReasoningEffort(
  value: ProviderReasoningEffort | null | undefined,
): ProviderReasoningEffort | null {
  switch (value) {
    case "low":
    case "medium":
    case "high":
    case "max":
      return value;
    default:
      return null;
  }
}

function normalizeProtocolArray(
  supportedProtocols: ProviderProtocol[] | null | undefined,
  endpoints: ProviderProtocolEndpoint[],
  legacyProtocol: ProviderProtocol,
) {
  // 入口统一过 normalizeLegacyProtocol：旧值/未知值先折叠为规范名再去重。
  const fromList = (supportedProtocols ?? [])
    .map((value) => normalizeLegacyProtocol(value))
    .filter((value, index, list) => list.indexOf(value) === index);
  const fromEndpoints = endpoints
    .filter((item) => item.enabled)
    .map((item) => item.protocol);
  const merged = [...new Set([...fromList, ...fromEndpoints])];
  return merged.length ? merged : [normalizeLegacyProtocol(legacyProtocol)];
}

function normalizeEndpoints(
  provider: Pick<
    ProviderConfig,
    "protocol" | "baseUrl" | "authType"
  > &
    Partial<Pick<ProviderConfig, "endpoints" | "supportedProtocols">>,
): ProviderProtocolEndpoint[] {
  const defaults = getDefaultEndpoints();
  const providerProtocol = normalizeLegacyProtocol(provider.protocol);
  // 旧文件 endpoint 可能携带 legacy 协议名：先映射到规范名再合并（首个优先），
  // 防止 openai/openai-completions 双条目在规范化后重复出现。
  const existingByProtocol = new Map<ProviderProtocol, ProviderProtocolEndpoint>();
  for (const item of provider.endpoints ?? []) {
    const canonical = normalizeLegacyProtocol(item?.protocol);
    if (!existingByProtocol.has(canonical)) {
      existingByProtocol.set(canonical, { ...item, protocol: canonical });
    }
  }

  const entries = defaults.map((defaultEntry) => {
    const existing = existingByProtocol.get(defaultEntry.protocol);
    if (existing) {
      return {
        protocol: defaultEntry.protocol,
        enabled: Boolean(existing.enabled),
        baseUrl: existing.baseUrl?.trim() || defaultEntry.baseUrl,
        authType: existing.authType ?? defaultEntry.authType,
      };
    }

    const enabled =
      (provider.supportedProtocols ?? []).some(
        (value) => normalizeLegacyProtocol(value) === defaultEntry.protocol,
      ) || providerProtocol === defaultEntry.protocol;
    const baseUrl =
      providerProtocol === defaultEntry.protocol && provider.baseUrl?.trim()
        ? provider.baseUrl.trim()
        : defaultEntry.baseUrl;
    const authType =
      providerProtocol === defaultEntry.protocol
        ? provider.authType ?? defaultEntry.authType
        : defaultEntry.authType;

    return {
      ...defaultEntry,
      enabled,
      baseUrl,
      authType,
    };
  });

  return entries;
}

function getProviderProtocol(provider: ProviderConfig, model?: ProviderModelConfig | null) {
  if (model?.protocol) {
    return model.protocol;
  }

  const supported = provider.supportedProtocols.find((item) =>
    provider.endpoints.some((endpoint) => endpoint.protocol === item && endpoint.enabled),
  );
  return supported ?? provider.protocol;
}

function normalizeCapabilitiesForPersistence(
  capabilities: ProviderModelCapabilities,
): ProviderModelCapabilities {
  return {
    contextWindowTokens:
      normalizeNullablePositiveInteger(capabilities.contextWindowTokens) ??
      DEFAULT_CONTEXT_WINDOW_TOKENS,
    supportsTools: capabilities.supportsTools ?? true,
    supportsStreaming: capabilities.supportsStreaming ?? true,
    supportsReasoning: capabilities.supportsReasoning,
    supportsImageInput: capabilities.supportsImageInput,
    supportsVideoInput: capabilities.supportsVideoInput,
    supportsAudioInput: capabilities.supportsAudioInput,
    supportsTextOutput: capabilities.supportsTextOutput ?? true,
    supportsImageOutput: capabilities.supportsImageOutput,
    supportsVideoOutput: capabilities.supportsVideoOutput,
    supportsAudioOutput: capabilities.supportsAudioOutput,
  };
}

export function resolveCapabilityDeclaration(
  protocol: ProviderProtocol,
  modelIdValue: string,
  preset: ProviderCapabilityPresetId,
  capabilities: Partial<ProviderModelCapabilities> | null | undefined,
): ProviderModelCapabilityDeclaration {
  if (preset !== "custom") {
    return {
      capabilityPreset: preset,
      capabilities: normalizeCapabilitiesForPersistence(
        capabilitiesForPreset(preset, protocol, modelIdValue),
      ),
    };
  }

  const normalized = getSafeCapabilities(capabilities);

  return {
    capabilityPreset: preset,
    capabilities: normalizeCapabilitiesForPersistence({
      ...normalized,
      contextWindowTokens: normalizeNullablePositiveInteger(
        normalized.contextWindowTokens,
      ),
    }),
  };
}

export function resolveModelCapabilityDeclaration(
  model: Pick<
    ProviderModelConfig,
    "model" | "capabilityPreset" | "capabilities" | "protocol"
  >,
  protocol: ProviderProtocol,
): ProviderModelCapabilityDeclaration {
  const resolvedProtocol = model.protocol ?? protocol;
  const capabilityPreset = normalizeCapabilityPresetId(
    model.capabilityPreset,
    resolvedProtocol,
    model.model,
  );
  return resolveCapabilityDeclaration(
    resolvedProtocol,
    model.model,
    capabilityPreset,
    model.capabilities,
  );
}

export function resolveModelUserPolicy(
  model: Pick<
    ProviderModelConfig,
    | "temperature"
    | "maxOutputTokens"
    | "reasoningEffort"
    | "reasoningBudgetTokens"
  >,
  capabilities: ProviderModelCapabilities,
): ProviderModelUserPolicy {
  return {
    temperature: model.temperature ?? 0,
    maxOutputTokens:
      normalizeNullablePositiveInteger(model.maxOutputTokens) ??
      DEFAULT_MAX_OUTPUT_TOKENS,
    reasoningEffort: capabilities.supportsReasoning
      ? normalizeReasoningEffort(model.reasoningEffort)
      : null,
    reasoningBudgetTokens: capabilities.supportsReasoning
      ? normalizeNullablePositiveInteger(model.reasoningBudgetTokens)
      : null,
  };
}

export function buildProviderModelConfig(
  identity: ProviderModelIdentity,
  declaration: ProviderModelCapabilityDeclaration,
  userPolicy: ProviderModelUserPolicy,
): ProviderModelConfig {
  // baseUrl 单点清洗：空白折叠为 null（= 继承），其余 trim 后透传。
  const rawBaseUrl = identity.baseUrl;
  const baseUrl =
    typeof rawBaseUrl === "string" ? rawBaseUrl.trim() || null : (rawBaseUrl ?? null);
  return {
    ...identity,
    protocol: identity.protocol ?? null,
    baseUrl,
    ...declaration,
    ...userPolicy,
  };
}

function createEmptyModel(): ProviderModelConfig {
  return buildProviderModelConfig(
    {
      id: createId("model"),
      name: "",
      model: "",
      protocol: "openai-completions",
      baseUrl: null,
    },
    {
      capabilityPreset: "custom",
      capabilities: createDefaultCapabilities(),
    },
    createDefaultModelUserPolicy(),
  );
}

function createEmptyProvider(): ProviderConfig {
  const model = createEmptyModel();
  const name = "new-provider";
  const endpoints = getDefaultEndpoints();

  return {
    id: createId("provider"),
    name,
    protocol: "openai-completions",
    baseUrl: endpoints[0]?.baseUrl ?? defaultBaseUrlFor("openai-completions"),
    authType: "auto",
    supportedProtocols: ["openai-completions"],
    endpoints,
    apiKeyEnvVar: deriveEnvVarName(name),
    apiKeyValue: "",
    apiKeyPresent: false,
    models: [model],
    selectedModelId: model.id,
  };
}

function createPresetProvider(
  id: string,
  name: string,
  protocol: ProviderProtocol,
  baseUrl: string,
  modelName: string,
  modelIdValue: string,
): ProviderConfig {
  const modelId = createId("model");
  const endpoints = getDefaultEndpoints().map((item) =>
    item.protocol === protocol
      ? {
          ...item,
          enabled: true,
          baseUrl,
          authType: "auto" as ProviderAuthType,
        }
      : item,
  );

  return {
    id,
    name,
    protocol,
    baseUrl,
    authType: "auto",
    supportedProtocols: [protocol],
    endpoints,
    apiKeyEnvVar: deriveEnvVarName(name),
    apiKeyValue: "",
    apiKeyPresent: false,
    models: [
      buildProviderModelConfig(
        {
          id: modelId,
          name: modelName,
          model: modelIdValue,
          protocol,
        },
        resolveCapabilityDeclaration(
          protocol,
          modelIdValue,
          inferCapabilityPreset(protocol, modelIdValue),
          null,
        ),
        {
          ...createDefaultModelUserPolicy(),
          temperature: 0.2,
        },
      ),
    ],
    selectedModelId: modelId,
  };
}

function createDeepseekV4ProModel(): ProviderModelConfig {
  return buildProviderModelConfig(
    {
      id: createId("model"),
      name: "DeepSeek V4 Pro",
      model: "deepseek-v4-pro",
      protocol: "openai-completions",
    },
    resolveCapabilityDeclaration(
      "openai-completions",
      "deepseek-v4-pro",
      "deepseek-reasoner",
      null,
    ),
    {
      ...createDefaultModelUserPolicy(),
      temperature: 0.2,
      reasoningEffort: "medium",
    },
  );
}

function isDeepseekV4ProModelValue(modelValue: string) {
  const lower = modelValue.trim().toLowerCase();
  return lower === "deepseek-v4-pro" || lower.endsWith("/deepseek-v4-pro");
}

function modelUniquenessKey(modelValue: string) {
  if (isDeepseekV4ProModelValue(modelValue)) {
    return "deepseek-v4-pro";
  }
  return modelValue.trim().toLowerCase();
}

function dedupeModelsWithinProvider(
  models: ProviderModelConfig[],
  selectedModelId: string | null,
) {
  const seen = new Map<string, ProviderModelConfig>();
  let selectedReplacementId: string | null = null;
  const deduped = models.filter((model) => {
    const key = modelUniquenessKey(model.model);
    const existing = seen.get(key);
    if (!existing) {
      seen.set(key, model);
      return true;
    }
    if (model.id === selectedModelId) {
      selectedReplacementId = existing.id;
    }
    return false;
  });

  const nextSelectedModelId =
    selectedModelId && deduped.some((model) => model.id === selectedModelId)
      ? selectedModelId
      : selectedReplacementId ?? deduped[0]?.id ?? null;

  return {
    models: deduped,
    selectedModelId: nextSelectedModelId,
  };
}

function normalizeModel(
  provider: ProviderConfig,
  model: ProviderModelConfig,
): ProviderModelConfig {
  const protocol = getProviderProtocol(provider, model);
  const declaration = resolveModelCapabilityDeclaration(model, protocol);
  const userPolicy = resolveModelUserPolicy(model, declaration.capabilities);

  return buildProviderModelConfig(
    {
      id: model.id,
      name: model.name,
      model: model.model,
      protocol,
      baseUrl: model.baseUrl ?? null,
    },
    declaration,
    userPolicy,
  );
}

function normalizeProvider(provider: ProviderConfig): ProviderConfig {
  // D1 单点收敛：provider.protocol 先过 legacy 规范化，endpoints/supportedProtocols
  // 的映射在各自入口完成，models 在 normalizeModel 内收敛。
  const normalized: ProviderConfig = {
    ...provider,
    protocol: normalizeLegacyProtocol(provider.protocol),
  };
  const endpoints = normalizeEndpoints(normalized);
  const supportedProtocols = normalizeProtocolArray(
    normalized.supportedProtocols,
    endpoints,
    normalized.protocol,
  );
  const primaryProtocol = supportedProtocols[0] ?? normalized.protocol;
  const primaryEndpoint =
    endpoints.find((item) => item.protocol === primaryProtocol) ?? endpoints[0];

  const baseProvider: ProviderConfig = {
    ...normalized,
    protocol: primaryProtocol,
    baseUrl: primaryEndpoint?.baseUrl ?? defaultBaseUrlFor(primaryProtocol),
    authType: primaryEndpoint?.authType ?? "auto",
    supportedProtocols,
    endpoints,
  };

  let models = baseProvider.models.map((model) => normalizeModel(baseProvider, model));
  let selectedModelId = baseProvider.selectedModelId;
  if (baseProvider.id === "provider-deepseek") {
    if (!models.some((model) => isDeepseekV4ProModelValue(model.model))) {
      models.push(normalizeModel(baseProvider, createDeepseekV4ProModel()));
    }
  }
  ({ models, selectedModelId } = dedupeModelsWithinProvider(models, selectedModelId));

  return {
    ...baseProvider,
    apiKeyEnvVar: deriveEnvVarName(baseProvider.name),
    models,
    selectedModelId,
  };
}

function createBrowserRegistry(): ProviderRegistry {
  const ppx = createPresetProvider(
    "provider-ppx",
    "ppx",
    "openai-completions",
    "https://api.psydo.top/v1",
    "GPT 5.4",
    "gpt-5.4",
  );
  const openrouter = createPresetProvider(
    "provider-openrouter",
    "openrouter",
    "openai-completions",
    "https://openrouter.ai/api/v1",
    "OpenAI GPT-4.1 Mini",
    "openai/gpt-4.1-mini",
  );
  const deepseek = createPresetProvider(
    "provider-deepseek",
    "deepseek",
    "openai-completions",
    "https://api.deepseek.com/v1",
    "DeepSeek V4 Flash",
    "deepseek-v4-flash",
  );
  deepseek.models.push(createDeepseekV4ProModel());

  return {
    providers: [ppx, deepseek, openrouter].map(normalizeProvider),
    selectedProviderId: ppx.id,
  };
}

export type FetchModelCatalogInput = {
  providerId?: string | null;
  protocol: ProviderProtocol;
  /** 拉取用 Base URL：空/空白时按空串传给后端（后端按协议默认值解析）。 */
  baseUrl?: string | null;
  /** 表单中未保存的 API Key；缺省时由后端按 providerId 解析已存密钥。 */
  apiKey?: string | null;
};

export type AddModelsFromCatalogOptions = {
  protocol: ProviderProtocol;
  baseUrl?: string | null;
};

type ProviderState = {
  registry: ProviderRegistry | null;
  selectedReasoningEffort: ProviderReasoningEffort | null;
  loading: boolean;
  saving: boolean;
  error: string | null;
  notice: string | null;
  loadingModels: boolean;
  catalogError: string | null;
  catalogModels: string[];
};

export const useProviderStore = defineStore("providers", {
  state: (): ProviderState => ({
    registry: null,
    selectedReasoningEffort: null,
    loading: false,
    saving: false,
    error: null,
    notice: null,
    loadingModels: false,
    catalogError: null,
    catalogModels: [],
  }),
  getters: {
    providers(state): ProviderConfig[] {
      return state.registry?.providers ?? [];
    },
    currentProvider(state): ProviderConfig | null {
      if (!state.registry) {
        return null;
      }

      const provider =
        state.registry.providers.find(
          (item) => item.id === state.registry?.selectedProviderId,
        ) ?? state.registry.providers[0];

      return provider ?? null;
    },
    currentModel(): ProviderModelConfig | null {
      if (!this.currentProvider) {
        return null;
      }

      return (
        this.currentProvider.models.find(
          (model) => model.id === this.currentProvider?.selectedModelId,
        ) ??
        this.currentProvider.models[0] ??
        null
      );
    },
    currentReasoningEffort(state): ProviderReasoningEffort | null {
      return state.selectedReasoningEffort;
    },
  },
  actions: {
    syncReasoningEffortFromCurrentModel() {
      const currentModel = this.currentModel;
      const supportsReasoning =
        currentModel?.capabilities?.supportsReasoning ?? false;
      this.selectedReasoningEffort = supportsReasoning
        ? (currentModel?.reasoningEffort ?? null)
        : null;
    },
    clearNotice() {
      this.notice = null;
    },
    async loadRegistry() {
      this.loading = true;
      this.error = null;

      try {
        if (!isTauriAvailable()) {
          this.registry = createBrowserRegistry();
          this.notice =
            "当前是浏览器预览模式，模型配置只用于界面预览，不会写入本地 providers.json。";
          return;
        }

        const registry = await safeInvoke<ProviderRegistry>(
          "load_provider_registry",
        );
        this.registry = {
          ...registry,
          providers: registry.providers.map(normalizeProvider),
        };
        this.syncReasoningEffortFromCurrentModel();
      } catch (error) {
        this.selectedReasoningEffort = null;
        this.error = `加载模型配置失败：${String(error)}`;
      } finally {
        this.loading = false;
      }
    },
    async saveRegistry() {
      if (!this.registry) {
        return;
      }

      this.registry.providers = this.registry.providers.map(normalizeProvider);

      this.saving = true;
      this.error = null;
      this.notice = null;

      try {
        if (!isTauriAvailable()) {
          this.notice =
            "当前是浏览器预览模式，保存结果只保留在当前页面会话中。";
          return;
        }

        const registry = await safeInvoke<ProviderRegistry>(
          "save_provider_registry",
          {
            registry: this.registry,
          },
        );
        this.registry = {
          ...registry,
          providers: registry.providers.map(normalizeProvider),
        };
        this.syncReasoningEffortFromCurrentModel();
        this.notice = "提供商配置已保存；敏感密钥已写入应用密钥存储。";
      } catch (error) {
        this.error = `保存模型配置失败：${String(error)}`;
      } finally {
        this.saving = false;
      }
    },
    selectProvider(providerId: string) {
      if (!this.registry) {
        return;
      }

      this.registry.selectedProviderId = providerId;
      this.syncReasoningEffortFromCurrentModel();
    },
    selectModel(providerId: string, modelId: string) {
      if (!this.registry) {
        return;
      }

      const provider = this.registry.providers.find(
        (item) => item.id === providerId,
      );
      if (!provider) {
        return;
      }

      provider.selectedModelId = modelId;
      this.registry.selectedProviderId = providerId;
      this.syncReasoningEffortFromCurrentModel();
    },
    setCurrentReasoningEffort(value: ProviderReasoningEffort | null) {
      this.selectedReasoningEffort = value;
    },
    addProvider() {
      if (!this.registry) {
        this.registry = {
          providers: [],
          selectedProviderId: null,
        };
      }

      const provider = createEmptyProvider();
      this.registry.providers.push(provider);
      this.registry.selectedProviderId = provider.id;
      return provider.id;
    },
    removeProvider(providerId: string) {
      if (!this.registry) {
        return;
      }

      this.registry.providers = this.registry.providers.filter(
        (item) => item.id !== providerId,
      );

      if (this.registry.selectedProviderId === providerId) {
        this.registry.selectedProviderId =
          this.registry.providers[0]?.id ?? null;
      }
    },
    updateProviderField<K extends keyof ProviderConfig>(
      providerId: string,
      key: K,
      value: ProviderConfig[K],
    ) {
      const provider = this.registry?.providers.find(
        (item) => item.id === providerId,
      );
      if (!provider) {
        return;
      }

      provider[key] = value;

      if (key === "name") {
        provider.apiKeyEnvVar = deriveEnvVarName(String(value));
      }
    },
    addModel(providerId: string) {
      const provider = this.registry?.providers.find(
        (item) => item.id === providerId,
      );
      if (!provider) {
        return;
      }

      const protocol = provider.supportedProtocols[0] ?? provider.protocol;
      const model = buildProviderModelConfig(
        {
          ...createEmptyModel(),
          id: createId("model"),
          protocol,
        },
        {
          capabilityPreset: "custom",
          capabilities: createDefaultCapabilities(),
        },
        createDefaultModelUserPolicy(),
      );
      provider.models.push(model);
      provider.selectedModelId = model.id;
    },
    upsertModel(providerId: string, payload: ProviderModelConfig) {
      const provider = this.registry?.providers.find(
        (item) => item.id === providerId,
      );
      if (!provider) {
        return null;
      }

      const normalizedPayload = normalizeModel(provider, payload);
      const duplicate = provider.models.find(
        (item) =>
          item.id !== normalizedPayload.id &&
          modelUniquenessKey(item.model) === modelUniquenessKey(normalizedPayload.model),
      );
      if (duplicate) {
        this.error = `同一提供商下已存在模型：${normalizedPayload.model}`;
        return null;
      }

      const index = provider.models.findIndex(
        (item) => item.id === normalizedPayload.id,
      );

      if (index >= 0) {
        provider.models[index] = normalizedPayload;
      } else {
        provider.models.push(normalizedPayload);
      }

      if (!provider.selectedModelId) {
        provider.selectedModelId = normalizedPayload.id;
      }
      this.error = null;
      return normalizedPayload;
    },
    updateModelField<K extends keyof ProviderModelConfig>(
      providerId: string,
      modelId: string,
      key: K,
      value: ProviderModelConfig[K],
    ) {
      const provider = this.registry?.providers.find(
        (item) => item.id === providerId,
      );
      const model = provider?.models.find((item) => item.id === modelId);

      if (!model) {
        return;
      }

      model[key] = value;
    },
    removeModel(providerId: string, modelId: string) {
      const provider = this.registry?.providers.find(
        (item) => item.id === providerId,
      );
      if (!provider) {
        return;
      }

      provider.models = provider.models.filter((item) => item.id !== modelId);

      if (provider.selectedModelId === modelId) {
        provider.selectedModelId = provider.models[0]?.id ?? null;
      }

      if (provider.models.length === 0) {
        const model = createEmptyModel();
        provider.models = [model];
        provider.selectedModelId = model.id;
      }
    },
    clearModelCatalog() {
      this.catalogModels = [];
      this.catalogError = null;
    },
    // D3/D6：模型目录拉取。浏览器预览模式由 isTauriAvailable 守卫直接提示、不触网；
    // Tauri 环境走 fetch_provider_models 命令，失败进 catalogError（不占用全局 error 横幅）。
    async fetchModelCatalog(input: FetchModelCatalogInput): Promise<string[]> {
      if (!isTauriAvailable()) {
        this.catalogModels = [];
        this.catalogError = null;
        this.notice = "预览模式不可用拉取模型列表；请在桌面应用内使用获取列表。";
        return [];
      }

      this.loadingModels = true;
      this.catalogError = null;
      try {
        const models = await safeInvoke<string[]>("fetch_provider_models", {
          providerId: input.providerId ?? null,
          protocol: input.protocol,
          baseUrl: input.baseUrl?.trim() || "",
          apiKey: input.apiKey?.trim() ? input.apiKey.trim() : null,
        });
        const list = Array.isArray(models)
          ? [...new Set(models.filter((item): item is string => typeof item === "string" && item.trim().length > 0).map((item) => item.trim()))]
          : [];
        this.catalogModels = list;
        return list;
      } catch (error) {
        this.catalogModels = [];
        this.catalogError = `拉取模型列表失败：${String(error)}`;
        return [];
      } finally {
        this.loadingModels = false;
      }
    },
    // D6：目录批量添加专用通道——预去重后批量 upsert，返回 {added, skipped}，
    // 不复用 upsertModel 的 this.error 错误通道（避免全局红色横幅误报）。
    addModelsFromCatalog(
      providerId: string,
      modelIds: string[],
      options: AddModelsFromCatalogOptions,
    ): { added: number; skipped: number; lastAddedModelId: string | null } {
      const provider = this.registry?.providers.find(
        (item) => item.id === providerId,
      );
      if (!provider) {
        return { added: 0, skipped: modelIds.length, lastAddedModelId: null };
      }

      const existingKeys = new Set(
        provider.models.map((model) => modelUniquenessKey(model.model)),
      );
      const batchKeys = new Set<string>();
      let added = 0;
      let skipped = 0;
      let lastAddedModelId: string | null = null;

      for (const rawId of modelIds) {
        const modelIdValue = String(rawId ?? "").trim();
        if (!modelIdValue) {
          skipped += 1;
          continue;
        }
        const key = modelUniquenessKey(modelIdValue);
        // 同 ID 按别名折叠：与既有模型冲突或批次内重复都计为跳过。
        if (existingKeys.has(key) || batchKeys.has(key)) {
          skipped += 1;
          continue;
        }

        batchKeys.add(key);
        const model = normalizeModel(
          provider,
          buildProviderModelConfig(
            {
              id: createId("model"),
              name: modelIdValue,
              model: modelIdValue,
              protocol: options.protocol,
              baseUrl: options.baseUrl ?? null,
            },
            {
              capabilityPreset: "custom",
              capabilities: createDefaultCapabilities(),
            },
            createDefaultModelUserPolicy(),
          ),
        );
        provider.models.push(model);
        existingKeys.add(key);
        added += 1;
        lastAddedModelId = model.id;
      }

      if (!provider.selectedModelId) {
        provider.selectedModelId =
          provider.models[provider.models.length - 1]?.id ?? null;
      }

      return { added, skipped, lastAddedModelId };
    },
  },
});
