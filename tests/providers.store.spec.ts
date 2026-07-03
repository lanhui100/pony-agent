import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import {
  buildProviderModelConfig,
  createDefaultCapabilities,
  createDefaultModelUserPolicy,
  resolveCapabilityDeclaration,
  resolveModelCapabilityDeclaration,
  resolveModelUserPolicy,
  useProviderStore,
} from "@/stores/providers";

const tauriMocks = vi.hoisted(() => ({
  mockIsTauriAvailable: vi.fn(),
  mockSafeInvoke: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  isTauriAvailable: tauriMocks.mockIsTauriAvailable,
  safeInvoke: tauriMocks.mockSafeInvoke,
}));

describe("provider capability layering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
  });

  it("keeps capability declaration as model facts while auto preset still infers concrete facts", () => {
    const declaration = resolveModelCapabilityDeclaration(
      {
        model: "gpt-5.4",
        capabilityPreset: "auto",
        capabilities: createDefaultCapabilities(),
      },
      "openai",
    );

    expect(declaration.capabilityPreset).toBe("auto");
    expect(declaration.capabilities.supportsReasoning).toBe(true);
    expect(declaration.capabilities.contextWindowTokens).toBe(256000);
  });

  it("normalizes user policy separately from capabilities", () => {
    const userPolicy = resolveModelUserPolicy(
      {
        temperature: 0.3,
        maxOutputTokens: 4096,
        reasoningEffort: "high",
        reasoningBudgetTokens: 2048,
      },
      {
        ...createDefaultCapabilities(),
        supportsReasoning: false,
      },
    );

    expect(userPolicy.temperature).toBe(0.3);
    expect(userPolicy.maxOutputTokens).toBe(4096);
    expect(userPolicy.reasoningEffort).toBeNull();
    expect(userPolicy.reasoningBudgetTokens).toBeNull();
  });

  it("builds a flat provider model config from identity, facts and policy layers", () => {
    const declaration = resolveCapabilityDeclaration(
      "openai",
      "gpt-4.1-mini",
      "open-ai-chat",
      null,
    );
    const model = buildProviderModelConfig(
      {
        id: "model-1",
        name: "GPT 4.1 Mini",
        model: "gpt-4.1-mini",
      },
      declaration,
      {
        ...createDefaultModelUserPolicy(),
        temperature: 0.2,
      },
    );

    expect(model).toMatchObject({
      id: "model-1",
      name: "GPT 4.1 Mini",
      model: "gpt-4.1-mini",
      capabilityPreset: "open-ai-chat",
      temperature: 0.2,
      maxOutputTokens: 64000,
    });
    expect(model.capabilities.supportsImageInput).toBe(true);
  });

  it("treats DeepSeek V4 Pro as a reasoning model instead of V4 Flash chat", () => {
    const declaration = resolveCapabilityDeclaration(
      "openai",
      "deepseek-v4-pro",
      "deepseek-reasoner",
      null,
    );
    const model = buildProviderModelConfig(
      {
        id: "model-deepseek-v4-pro",
        name: "DeepSeek V4 Pro",
        model: "deepseek-v4-pro",
        protocol: "openai",
      },
      declaration,
      {
        ...createDefaultModelUserPolicy(),
        reasoningEffort: "medium",
      },
    );

    expect(model.model).toBe("deepseek-v4-pro");
    expect(model.capabilityPreset).toBe("deepseek-reasoner");
    expect(model.capabilities.supportsReasoning).toBe(true);
    expect(model.reasoningEffort).toBe("medium");
  });

  it("deduplicates existing DeepSeek V4 Pro aliases when normalizing the registry", async () => {
    const store = useProviderStore();
    const flash = buildProviderModelConfig(
      {
        id: "model-deepseek-default",
        name: "DeepSeek V4 Flash",
        model: "deepseek-v4-flash",
        protocol: "openai",
      },
      resolveCapabilityDeclaration("openai", "deepseek-v4-flash", "deepseek-chat", null),
      createDefaultModelUserPolicy(),
    );
    const prefixedPro = buildProviderModelConfig(
      {
        id: "model-deepseek-v4-pro-prefixed",
        name: "DeepSeek V4 Pro",
        model: "deepseek/deepseek-v4-pro",
        protocol: "openai",
      },
      resolveCapabilityDeclaration("openai", "deepseek/deepseek-v4-pro", "deepseek-reasoner", null),
      {
        ...createDefaultModelUserPolicy(),
        reasoningEffort: "medium",
      },
    );
    const duplicatedDefaultPro = buildProviderModelConfig(
      {
        id: "model-deepseek-v4-pro",
        name: "DeepSeek V4 Pro",
        model: "deepseek-v4-pro",
        protocol: "openai",
      },
      resolveCapabilityDeclaration("openai", "deepseek-v4-pro", "deepseek-reasoner", null),
      {
        ...createDefaultModelUserPolicy(),
        reasoningEffort: "medium",
      },
    );

    store.$patch({
      registry: {
        selectedProviderId: "provider-deepseek",
        providers: [
          {
            id: "provider-deepseek",
            name: "deepseek",
            protocol: "openai",
            baseUrl: "https://api.deepseek.com/v1",
            authType: "auto",
            supportedProtocols: ["openai"],
            endpoints: [
              {
                protocol: "openai",
                enabled: true,
                baseUrl: "https://api.deepseek.com/v1",
                authType: "auto",
              },
            ],
            apiKeyEnvVar: "DEEPSEEK_API_KEY",
            apiKeyValue: "",
            apiKeyPresent: false,
            selectedModelId: "model-deepseek-v4-pro",
            models: [flash, prefixedPro, duplicatedDefaultPro],
          },
        ],
      },
    });

    await store.saveRegistry();

    const provider = store.providers[0]!;
    const proModels = provider.models.filter((model) => model.model.endsWith("deepseek-v4-pro"));
    expect(proModels).toHaveLength(1);
    expect(proModels[0]?.id).toBe("model-deepseek-v4-pro-prefixed");
    expect(provider.selectedModelId).toBe("model-deepseek-v4-pro-prefixed");
  });

  it("rejects duplicate model values within the same provider", () => {
    const store = useProviderStore();
    const existing = buildProviderModelConfig(
      {
        id: "model-gpt-5",
        name: "GPT-5",
        model: "gpt-5",
        protocol: "openai",
      },
      resolveCapabilityDeclaration("openai", "gpt-5", "open-ai-reasoning", null),
      createDefaultModelUserPolicy(),
    );
    const duplicate = buildProviderModelConfig(
      {
        id: "model-gpt-5-copy",
        name: "GPT-5 Copy",
        model: " GPT-5 ",
        protocol: "openai",
      },
      resolveCapabilityDeclaration("openai", "gpt-5", "open-ai-reasoning", null),
      createDefaultModelUserPolicy(),
    );

    store.$patch({
      registry: {
        selectedProviderId: "provider-openai",
        providers: [
          {
            id: "provider-openai",
            name: "openai",
            protocol: "openai",
            baseUrl: "https://api.openai.com/v1",
            authType: "auto",
            supportedProtocols: ["openai"],
            endpoints: [
              {
                protocol: "openai",
                enabled: true,
                baseUrl: "https://api.openai.com/v1",
                authType: "auto",
              },
            ],
            apiKeyEnvVar: "OPENAI_API_KEY",
            apiKeyValue: "",
            apiKeyPresent: false,
            selectedModelId: existing.id,
            models: [existing],
          },
        ],
      },
    });

    expect(store.upsertModel("provider-openai", duplicate)).toBeNull();
    expect(store.providers[0]?.models).toHaveLength(1);
    expect(store.error).toContain("同一提供商下已存在模型");
  });
});
