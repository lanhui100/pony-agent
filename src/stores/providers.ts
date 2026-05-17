import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import type { ProviderConfig, ProviderModelConfig, ProviderProtocol, ProviderRegistry } from "@/types/provider";

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
    maxOutputTokens: 0
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
        const registry = await invoke<ProviderRegistry>("load_provider_registry");
        this.registry = registry;
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
        apiKeyEnvVar: deriveEnvVarName(provider.name)
      }));

      this.saving = true;
      this.error = null;
      this.notice = null;

      try {
        const registry = await invoke<ProviderRegistry>("save_provider_registry", {
          registry: this.registry
        });
        this.registry = registry;
        this.notice = "模型配置已保存，API Key 已同步写入环境变量。";
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
    addProvider(protocol: ProviderProtocol = "openai") {
      if (!this.registry) {
        this.registry = {
          providers: [],
          selectedProviderId: null
        };
      }

      const provider = createEmptyProvider();
      provider.protocol = protocol;
      provider.baseUrl =
        protocol === "anthropic" ? "https://api.anthropic.com/v1" : "https://api.openai.com/v1";
      provider.name = protocol === "anthropic" ? "anthropic" : "openai";
      provider.apiKeyEnvVar = deriveEnvVarName(provider.name);
      provider.models[0].name = protocol === "anthropic" ? "Claude Sonnet" : "GPT 4.1 Mini";
      provider.models[0].model = protocol === "anthropic" ? "claude-3-7-sonnet-latest" : "gpt-4.1-mini";

      this.registry.providers.push(provider);
      this.registry.selectedProviderId = provider.id;
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

      const index = provider.models.findIndex((item) => item.id === payload.id);

      if (index >= 0) {
        provider.models[index] = payload;
      } else {
        provider.models.push(payload);
      }

      if (!provider.selectedModelId) {
        provider.selectedModelId = payload.id;
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
