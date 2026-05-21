import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type {
  ProviderConfig,
  ProviderModelCapabilities,
  ProviderModelConfig,
  ProviderReasoningEffort,
  ProviderRegistry
} from "@/types/provider";

const DEFAULT_MAX_OUTPUT_TOKENS = 8192;

function createDefaultCapabilities(): ProviderModelCapabilities {
  return {
    contextWindowTokens: null,
    supportsTools: true,
    supportsStreaming: true,
    supportsImageInput: false,
    supportsReasoning: false
  };
}

function createId(prefix: string) {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}-${crypto.randomUUID()}`;
  }

  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
}

function createEmptyModel(): ProviderModelConfig {
  return {
    id: createId("model"),
    name: "",
    model: "",
    temperature: 0,
    maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
    reasoningEffort: null,
    reasoningBudgetTokens: null,
    capabilities: createDefaultCapabilities()
  };
}

function createCapabilities(overrides: Partial<ProviderModelCapabilities>): ProviderModelCapabilities {
  return {
    ...createDefaultCapabilities(),
    ...overrides
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
    selectedModelId: model.id
  };
}

function createPresetProvider(
  id: string,
  name: string,
  baseUrl: string,
  modelName: string,
  modelIdValue: string
): ProviderConfig {
  const modelId = createId("model");

  return {
    id,
    name,
    protocol: "openai",
    baseUrl,
    apiKeyEnvVar: deriveEnvVarName(name),
    apiKeyValue: "",
    apiKeyPresent: false,
    models: [
      {
        id: modelId,
        name: modelName,
        model: modelIdValue,
        temperature: 0.2,
        maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
        reasoningEffort: null,
        reasoningBudgetTokens: null,
        capabilities: createCapabilities({
          contextWindowTokens: protocolAwareContextWindow(baseUrl),
          supportsImageInput: supportsImageInputByDefault(baseUrl, modelIdValue),
          supportsReasoning: supportsReasoningByDefault(modelIdValue)
        })
      }
    ],
    selectedModelId: modelId
  };
}

function protocolAwareContextWindow(baseUrl: string) {
  if (baseUrl.includes("anthropic")) {
    return 200000;
  }

  return 128000;
}

function supportsImageInputByDefault(baseUrl: string, modelIdValue: string) {
  const lower = `${baseUrl} ${modelIdValue}`.toLowerCase();
  return lower.includes("gpt-4.1") || lower.includes("claude") || lower.includes("vision");
}

function supportsReasoningByDefault(modelIdValue: string) {
  const lower = modelIdValue.toLowerCase();
  return lower.includes("o1") || lower.includes("o3") || lower.includes("reason") || lower.includes("claude-3-7");
}

function normalizeReasoningEffort(value: ProviderReasoningEffort | null | undefined): ProviderReasoningEffort | null {
  return value ?? null;
}

function normalizeCapabilities(
  capabilities: Partial<ProviderModelCapabilities> | null | undefined
): ProviderModelCapabilities {
  return createCapabilities(capabilities ?? {});
}

function normalizeModel(model: ProviderModelConfig): ProviderModelConfig {
  return {
    ...model,
    reasoningEffort: normalizeReasoningEffort(model.reasoningEffort),
    reasoningBudgetTokens: model.reasoningBudgetTokens ?? null,
    capabilities: normalizeCapabilities(model.capabilities)
  };
}

function createBrowserRegistry(): ProviderRegistry {
  const openrouter = createPresetProvider(
    "provider-openrouter",
    "openrouter",
    "https://openrouter.ai/api/v1",
    "OpenAI GPT-4.1 Mini",
    "openai/gpt-4.1-mini"
  );
  const deepseek = createPresetProvider(
    "provider-deepseek",
    "deepseek",
    "https://api.deepseek.com/v1",
    "DeepSeek Chat",
    "deepseek-chat"
  );

  return {
    providers: [openrouter, deepseek],
    selectedProviderId: openrouter.id
  };
}

type ProviderState = {
  registry: ProviderRegistry | null;
  loading: boolean;
  saving: boolean;
  error: string | null;
  notice: string | null;
};

export const useProviderStore = defineStore("providers", {
  state: (): ProviderState => ({
    registry: null,
    loading: false,
    saving: false,
    error: null,
    notice: null
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
        state.registry.providers.find((item) => item.id === state.registry?.selectedProviderId) ??
        state.registry.providers[0];

      return provider ?? null;
    },
    currentModel(): ProviderModelConfig | null {
      if (!this.currentProvider) {
        return null;
      }

      return (
        this.currentProvider.models.find((model) => model.id === this.currentProvider?.selectedModelId) ??
        this.currentProvider.models[0] ??
        null
      );
    }
  },
  actions: {
    clearNotice() {
      this.notice = null;
    },
    async loadRegistry() {
      this.loading = true;
      this.error = null;

      try {
        if (!isTauriAvailable()) {
          this.registry = createBrowserRegistry();
          this.notice = "当前是浏览器预览模式，模型配置只用于界面预览，不会写入本地 providers.json。";
          return;
        }

        const registry = await safeInvoke<ProviderRegistry>("load_provider_registry");
        this.registry = {
          ...registry,
          providers: registry.providers.map((provider) => ({
            ...provider,
            models: provider.models.map(normalizeModel)
          }))
        };
      } catch (error) {
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
        models: provider.models.map(normalizeModel)
      }));

      this.saving = true;
      this.error = null;
      this.notice = null;

      try {
        if (!isTauriAvailable()) {
          this.notice = "当前是浏览器预览模式，保存结果只保留在当前页面会话中。";
          return;
        }

        const registry = await safeInvoke<ProviderRegistry>("save_provider_registry", {
          registry: this.registry
        });
        this.registry = registry;
        this.notice = "模型配置已保存到本地 providers.json。";
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
    },
    selectModel(providerId: string, modelId: string) {
      if (!this.registry) {
        return;
      }

      const provider = this.registry.providers.find((item) => item.id === providerId);
      if (!provider) {
        return;
      }

      provider.selectedModelId = modelId;
      this.registry.selectedProviderId = providerId;
    },
    addProvider() {
      if (!this.registry) {
        this.registry = {
          providers: [],
          selectedProviderId: null
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

      this.registry.providers = this.registry.providers.filter((item) => item.id !== providerId);

      if (this.registry.selectedProviderId === providerId) {
        this.registry.selectedProviderId = this.registry.providers[0]?.id ?? null;
      }
    },
    updateProviderField<K extends keyof ProviderConfig>(providerId: string, key: K, value: ProviderConfig[K]) {
      const provider = this.registry?.providers.find((item) => item.id === providerId);
      if (!provider) {
        return;
      }

      provider[key] = value;

      if (key === "name") {
        provider.apiKeyEnvVar = deriveEnvVarName(String(value));
      }
    },
    addModel(providerId: string) {
      const provider = this.registry?.providers.find((item) => item.id === providerId);
      if (!provider) {
        return;
      }

      const model = createEmptyModel();
      provider.models.push(model);
      provider.selectedModelId = model.id;
    },
    upsertModel(providerId: string, payload: ProviderModelConfig) {
      const provider = this.registry?.providers.find((item) => item.id === providerId);
      if (!provider) {
        return;
      }

      const normalizedPayload = normalizeModel(payload);

      const index = provider.models.findIndex((item) => item.id === normalizedPayload.id);

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
      value: ProviderModelConfig[K]
    ) {
      const provider = this.registry?.providers.find((item) => item.id === providerId);
      const model = provider?.models.find((item) => item.id === modelId);

      if (!model) {
        return;
      }

      model[key] = value;
    },
    removeModel(providerId: string, modelId: string) {
      const provider = this.registry?.providers.find((item) => item.id === providerId);
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
    }
  }
});
