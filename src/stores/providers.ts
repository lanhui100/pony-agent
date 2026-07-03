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

type CapabilityFactCatalogEntry = {
  protocol?: ProviderProtocol;
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

export function defaultBaseUrlFor(protocol: ProviderProtocol) {
  return protocol === "anthropic"
    ? "https://api.anthropic.com/v1"
    : "https://api.openai.com/v1";
}

function defaultAuthTypeFor(protocol: ProviderProtocol): ProviderAuthType {
  return protocol === "anthropic" ? "x-api-key" : "bearer";
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
    createDefaultEndpoint("openai", { enabled: true, authType: "auto" }),
    createDefaultEndpoint("anthropic"),
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

  const matched = CAPABILITY_CATALOG.find(
    (entry) =>
      (!entry.protocol || entry.protocol === protocol) &&
      entry.patterns.some((pattern) => lower.includes(pattern)),
  );
  if (matched) {
    return matched.preset;
  }

  if (protocol === "anthropic") {
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
  const fromList = (supportedProtocols ?? []).filter(
    (value, index, list) =>
      (value === "openai" || value === "anthropic") &&
      list.indexOf(value) === index,
  );
  const fromEndpoints = endpoints
    .filter((item) => item.enabled)
    .map((item) => item.protocol);
  const merged = [...new Set([...fromList, ...fromEndpoints])];
  return merged.length ? merged : [legacyProtocol];
}

function normalizeEndpoints(
  provider: Pick<
    ProviderConfig,
    "protocol" | "baseUrl" | "authType"
  > &
    Partial<Pick<ProviderConfig, "endpoints" | "supportedProtocols">>,
): ProviderProtocolEndpoint[] {
  const defaults = getDefaultEndpoints();
  const entries = defaults.map((defaultEntry) => {
    const existing = provider.endpoints?.find(
      (item) => item.protocol === defaultEntry.protocol,
    );
    if (existing) {
      return {
        protocol: defaultEntry.protocol,
        enabled: Boolean(existing.enabled),
        baseUrl: existing.baseUrl?.trim() || defaultEntry.baseUrl,
        authType: existing.authType ?? defaultEntry.authType,
      };
    }

    const enabled =
      provider.supportedProtocols?.includes(defaultEntry.protocol) ??
      provider.protocol === defaultEntry.protocol;
    const baseUrl =
      provider.protocol === defaultEntry.protocol && provider.baseUrl?.trim()
        ? provider.baseUrl.trim()
        : defaultEntry.baseUrl;
    const authType =
      provider.protocol === defaultEntry.protocol
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
  return {
    ...identity,
    protocol: identity.protocol ?? null,
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
      protocol: "openai",
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
    protocol: "openai",
    baseUrl: endpoints[0]?.baseUrl ?? defaultBaseUrlFor("openai"),
    authType: "auto",
    supportedProtocols: ["openai"],
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
      protocol: "openai",
    },
    resolveCapabilityDeclaration("openai", "deepseek-v4-pro", "deepseek-reasoner", null),
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
    },
    declaration,
    userPolicy,
  );
}

function normalizeProvider(provider: ProviderConfig): ProviderConfig {
  const endpoints = normalizeEndpoints(provider);
  const supportedProtocols = normalizeProtocolArray(
    provider.supportedProtocols,
    endpoints,
    provider.protocol,
  );
  const primaryProtocol = supportedProtocols[0] ?? provider.protocol;
  const primaryEndpoint =
    endpoints.find((item) => item.protocol === primaryProtocol) ?? endpoints[0];

  const baseProvider: ProviderConfig = {
    ...provider,
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
    "openai",
    "https://api.psydo.top/v1",
    "GPT 5.4",
    "gpt-5.4",
  );
  const openrouter = createPresetProvider(
    "provider-openrouter",
    "openrouter",
    "openai",
    "https://openrouter.ai/api/v1",
    "OpenAI GPT-4.1 Mini",
    "openai/gpt-4.1-mini",
  );
  const deepseek = createPresetProvider(
    "provider-deepseek",
    "deepseek",
    "openai",
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

type ProviderState = {
  registry: ProviderRegistry | null;
  selectedReasoningEffort: ProviderReasoningEffort | null;
  loading: boolean;
  saving: boolean;
  error: string | null;
  notice: string | null;
};

export const useProviderStore = defineStore("providers", {
  state: (): ProviderState => ({
    registry: null,
    selectedReasoningEffort: null,
    loading: false,
    saving: false,
    error: null,
    notice: null,
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
  },
});
