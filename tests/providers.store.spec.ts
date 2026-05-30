import { describe, expect, it } from "vitest";
import {
  buildProviderModelConfig,
  createDefaultCapabilities,
  createDefaultModelUserPolicy,
  resolveCapabilityDeclaration,
  resolveModelCapabilityDeclaration,
  resolveModelUserPolicy,
} from "@/stores/providers";

describe("provider capability layering", () => {
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
    expect(declaration.capabilities.contextWindowTokens).toBe(128000);
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
      maxOutputTokens: 8192,
    });
    expect(model.capabilities.supportsImageInput).toBe(true);
  });
});
