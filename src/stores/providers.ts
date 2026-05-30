import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type {
  ProviderCapabilityPresetId,
  ProviderModelIdentity,
  ProviderModelCapabilityDeclaration,
  ProviderConfig,
  ProviderModelCapabilities,
  ProviderModelConfig,
  ProviderModelUserPolicy,
  ProviderReasoningEffort,
  ProviderProtocol,
  ProviderRegistry,
} from "@/types/provider";

const DEFAULT_MAX_OUTPUT_TOKENS = 8192;

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

export function createDefaultCapabilities(): ProviderModelCapabilities {
  return {
    contextWindowTokens: null,
    supportsTools: true,
    supportsStreaming: true,
    supportsImageInput: false,
    supportsReasoning: false,
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

function capabilitiesForPreset(
  preset: Exclude<ProviderCapabilityPresetId, "custom">,
  protocol: ProviderProtocol,
  modelIdValue: string,
): ProviderModelCapabilities {
  const inferredPreset =
    preset === "auto" ? inferCapabilityPreset(protocol, modelIdValue) : preset;

  switch (inferredPreset) {
    case "open-ai-chat":
      return createCapabilities({
        contextWindowTokens: 128000,
        supportsTools: true,
        supportsStreaming: true,
        supportsImageInput: true,
        supportsReasoning: false,
      });
    case "open-ai-reasoning":
      return createCapabilities({
        contextWindowTokens: 128000,
        supportsTools: true,
        supportsStreaming: true,
        supportsImageInput: false,
        supportsReasoning: true,
      });
    case "anthropic-thinking":
      return createCapabilities({
        contextWindowTokens: 200000,
        supportsTools: true,
        supportsStreaming: true,
        supportsImageInput: true,
        supportsReasoning: true,
      });
    case "deepseek-chat":
      return createCapabilities({
        contextWindowTokens: 128000,
        supportsTools: true,
        supportsStreaming: true,
        supportsImageInput: false,
        supportsReasoning: false,
      });
    case "deepseek-reasoner":
      return createCapabilities({
        contextWindowTokens: 128000,
        supportsTools: true,
        supportsStreaming: true,
        supportsImageInput: false,
        supportsReasoning: true,
      });
    case "auto":
      return createCapabilities({
        contextWindowTokens: protocol === "anthropic" ? 200000 : 128000,
        supportsTools: true,
        supportsStreaming: true,
        supportsImageInput:
          modelIdValue.toLowerCase().includes("gpt-4.1") ||
          modelIdValue.toLowerCase().includes("claude") ||
          modelIdValue.toLowerCase().includes("vision"),
        supportsReasoning:
          modelIdValue.toLowerCase().includes("gpt-5") ||
          modelIdValue.toLowerCase().includes("o1") ||
          modelIdValue.toLowerCase().includes("o3") ||
          modelIdValue.toLowerCase().includes("reason") ||
          modelIdValue.toLowerCase().includes("claude-3-7") ||
          modelIdValue.toLowerCase().includes("deepseek-r1") ||
          modelIdValue.toLowerCase().includes("deepseek-reasoner") ||
          modelIdValue.toLowerCase().includes("deepseek-v4-pro"),
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

function createEmptyModel(): ProviderModelConfig {
  return buildProviderModelConfig(
    {
      id: createId("model"),
      name: "",
      model: "",
    },
    {
      capabilityPreset: "auto",
      capabilities: createDefaultCapabilities(),
    },
    createDefaultModelUserPolicy(),
  );
}

function createCapabilities(
  overrides: Partial<ProviderModelCapabilities>,
): ProviderModelCapabilities {
  return {
    ...createDefaultCapabilities(),
    ...overrides,
  };
}

function deriveEnvVarName(providerName: string) {
  const envName = providerName
    .trim()
    .replace(/[^a-zA-Z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .toUpperCase();

  return envName ? `${envName}_API_KEY` : "CUSTOM_PROVIDER_API_KEY";
}

function createEmptyProvider(): ProviderConfig {
  const model = createEmptyModel();
  const name = "new-provider";

  return {
    id: createId("provider"),
    name,
    protocol: "openai",
    baseUrl: "https://api.openai.com/v1",
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
  const declaration = resolveCapabilityDeclaration(
    protocol,
    modelIdValue,
    inferCapabilityPreset(protocol, modelIdValue),
    null,
  );

  return {
    id,
    name,
    protocol,
    baseUrl,
    apiKeyEnvVar: deriveEnvVarName(name),
    apiKeyValue: "",
    apiKeyPresent: false,
    models: [
      buildProviderModelConfig(
        {
          id: modelId,
          name: modelName,
          model: modelIdValue,
        },
        declaration,
        {
          ...createDefaultModelUserPolicy(),
          temperature: 0.2,
        },
      ),
    ],
    selectedModelId: modelId,
  };
}

function normalizeReasoningEffort(
  value: ProviderReasoningEffort | null | undefined,
): ProviderReasoningEffort | null {
  return value ?? null;
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
      capabilities: capabilitiesForPreset(preset, protocol, modelIdValue),
    };
  }

  const normalized = getSafeCapabilities(capabilities);

  return {
    capabilityPreset: preset,
    capabilities: {
      contextWindowTokens: normalizeNullablePositiveInteger(
        normalized.contextWindowTokens,
      ),
      supportsTools: normalized.supportsTools,
      supportsStreaming: normalized.supportsStreaming,
      supportsImageInput: normalized.supportsImageInput,
      supportsReasoning: normalized.supportsReasoning,
    },
  };
}

export function resolveModelCapabilityDeclaration(
  model: Pick<
    ProviderModelConfig,
    "model" | "capabilityPreset" | "capabilities"
  >,
  protocol: ProviderProtocol,
): ProviderModelCapabilityDeclaration {
  const capabilityPreset = normalizeCapabilityPresetId(
    model.capabilityPreset,
    protocol,
    model.model,
  );
  return resolveCapabilityDeclaration(
    protocol,
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
    temperature: model.temperature,
    maxOutputTokens: model.maxOutputTokens,
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
    ...declaration,
    ...userPolicy,
  };
}

function normalizeModel(
  model: ProviderModelConfig,
  protocol: ProviderProtocol,
): ProviderModelConfig {
  const declaration = resolveModelCapabilityDeclaration(model, protocol);
  const userPolicy = resolveModelUserPolicy(model, declaration.capabilities);

  return buildProviderModelConfig(
    {
      id: model.id,
      name: model.name,
      model: model.model,
    },
    declaration,
    userPolicy,
  );
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

  return {
    providers: [ppx, deepseek, openrouter],
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
          providers: registry.providers.map((provider) => ({
            ...provider,
            models: provider.models.map((model) =>
              normalizeModel(model, provider.protocol),
            ),
          })),
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

      this.registry.providers = this.registry.providers.map((provider) => ({
        ...provider,
        apiKeyEnvVar: deriveEnvVarName(provider.name),
        models: provider.models.map((model) =>
          normalizeModel(model, provider.protocol),
        ),
      }));

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
          providers: registry.providers.map((provider) => ({
            ...provider,
            models: provider.models.map((model) =>
              normalizeModel(model, provider.protocol),
            ),
          })),
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

      const model = createEmptyModel();
      provider.models.push(model);
      provider.selectedModelId = model.id;
    },
    upsertModel(providerId: string, payload: ProviderModelConfig) {
      const provider = this.registry?.providers.find(
        (item) => item.id === providerId,
      );
      if (!provider) {
        return;
      }

      const normalizedPayload = normalizeModel(payload, provider.protocol);

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
