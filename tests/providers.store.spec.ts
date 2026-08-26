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

describe("provider protocol tri-value layer (D1/D3/D6)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
  });

  it("maps legacy protocol values to canonical names and passes canonical values through", async () => {
    const { normalizeLegacyProtocol } = await import("@/stores/providers");
    expect(normalizeLegacyProtocol("openai")).toBe("openai-completions");
    expect(normalizeLegacyProtocol("anthropic")).toBe("anthropic-messages");
    expect(normalizeLegacyProtocol("openai-responses")).toBe("openai-responses");
    expect(normalizeLegacyProtocol("openai-completions")).toBe("openai-completions");
    expect(normalizeLegacyProtocol("anthropic-messages")).toBe("anthropic-messages");
    expect(normalizeLegacyProtocol("unknown-protocol")).toBe("openai-completions");

    // normalizeProvider 单点收敛：registry 载入路径上的旧值全部折叠为规范名
    const store = useProviderStore();
    store.$patch({
      registry: {
        selectedProviderId: "p1",
        providers: [
          {
            id: "p1",
            name: "legacy",
            protocol: "openai",
            baseUrl: "https://example.invalid/v1",
            authType: "auto",
            supportedProtocols: ["openai", "anthropic"],
            endpoints: [
              { protocol: "openai", enabled: true, baseUrl: "https://example.invalid/v1", authType: "auto" },
              { protocol: "anthropic", enabled: false, baseUrl: "https://api.anthropic.com/v1", authType: "x-api-key" },
            ],
            apiKeyEnvVar: "LEGACY_API_KEY",
            apiKeyValue: "",
            apiKeyPresent: false,
            models: [],
            selectedModelId: null,
          },
        ] as never,
      },
    });

    const provider = store.registry?.providers[0]!;
    // 生产链路经 loadRegistry/saveRegistry 的 normalizeProvider 单点收敛；
    // 浏览器分支的 saveRegistry 在触网检查前完成归一化，正好可作无网络验证。
    await store.saveRegistry();

    const normalized = store.registry?.providers[0]!;
    expect(normalized.protocol).toBe("openai-completions");
    expect(normalized.supportedProtocols).toEqual(["openai-completions", "anthropic-messages"]);
    expect(
      normalized.endpoints.map((endpoint) => endpoint.protocol),
    ).toEqual(["openai-completions", "openai-responses", "anthropic-messages"]);
    void provider;
  });

  it("guards model catalog fetch in browser preview without touching the network", async () => {
    const store = useProviderStore();
    const models = await store.fetchModelCatalog({
      providerId: "p1",
      protocol: "openai-completions",
      baseUrl: "https://example.invalid/v1",
    });

    expect(models).toEqual([]);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalled();
    expect(store.notice).toContain("预览模式不可用拉取模型列表");
    expect(store.catalogModels).toEqual([]);
  });

  it("forwards explicit args to fetch_provider_models and surfaces failures via catalogError", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);

    const store = useProviderStore();
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      expect(command).toBe("fetch_provider_models");
      // D6：apiKey 显式入参透传（未提供时为 null），baseUrl 原样传递。
      expect(args).toEqual({
        providerId: "p1",
        protocol: "anthropic-messages",
        baseUrl: "https://gateway.example.invalid/v1",
        apiKey: "sk-draft-key",
      });
      return ["m-a", "m-b"];
    });

    const models = await store.fetchModelCatalog({
      providerId: "p1",
      protocol: "anthropic-messages",
      baseUrl: "https://gateway.example.invalid/v1",
      apiKey: "sk-draft-key",
    });
    expect(models).toEqual(["m-a", "m-b"]);
    expect(store.catalogModels).toEqual(["m-a", "m-b"]);
    expect(store.catalogError).toBeNull();

    tauriMocks.mockSafeInvoke.mockRejectedValueOnce(new Error("401 unauthorized"));
    const failed = await store.fetchModelCatalog({
      providerId: "p1",
      protocol: "anthropic-messages",
      baseUrl: "https://gateway.example.invalid/v1",
    });
    expect(failed).toEqual([]);
    expect(store.catalogError).toContain("拉取模型列表失败");
    expect(store.error).toBeNull();
  });

  it("batch-adds catalog models with dedupe report and alias folding (D6)", async () => {
    const store = useProviderStore();
    seedSingleProvider(store, "https://example.invalid/v1");

    const result = store.addModelsFromCatalog(
      "provider-x",
      ["m1", " m2 ", "m2", "deepseek-v4-pro", "DEEPSEEK-V4-PRO"],
      { protocol: "openai-completions", baseUrl: "  https://mirror.example.invalid/v1  " },
    );

    const provider = store.registry?.providers.find((item) => item.id === "provider-x")!;
    const addedValues = provider.models.map((model) => model.model);

    // 批内重复与 deepseek 别名折叠均计入 skipped；空白项跳过。
    expect(result).toMatchObject({ added: 3, skipped: 2 });
    expect(addedValues).toContain("m1");
    expect(addedValues).toContain("m2");
    expect(result.lastAddedModelId).toBe(provider.models[provider.models.length - 1]?.id);

    const mirror = provider.models.find((model) => model.model === "m1");
    expect(mirror?.baseUrl).toBe("https://mirror.example.invalid/v1");
    expect(mirror?.protocol).toBe("openai-completions");

    // 不复用 upsertModel 的 error 通道。
    expect(store.error).toBeNull();
  });
});

function seedSingleProvider(store: ReturnType<typeof useProviderStore>, baseUrl: string) {
  store.$patch({
    registry: {
      selectedProviderId: "provider-x",
      providers: [
        {
          id: "provider-x",
          name: "X",
          protocol: "openai-completions",
          baseUrl,
          authType: "auto",
          supportedProtocols: ["openai-completions"],
          endpoints: [
            { protocol: "openai-completions", enabled: true, baseUrl, authType: "auto" },
          ],
          apiKeyEnvVar: "X_API_KEY",
          apiKeyValue: "",
          apiKeyPresent: false,
          models: [],
          selectedModelId: null,
        },
      ] as never,
    },
  });
}
