export type ProviderProtocol = "openai" | "anthropic";

export type ProviderReasoningEffort = "minimal" | "low" | "medium" | "high";

export type ProviderCapabilityPresetId =
  | "auto"
  | "open-ai-chat"
  | "open-ai-reasoning"
  | "anthropic-thinking"
  | "deepseek-chat"
  | "deepseek-reasoner"
  | "custom";

export type ProviderModelCapabilities = {
  contextWindowTokens: number | null;
  supportsTools: boolean;
  supportsStreaming: boolean;
  supportsImageInput: boolean;
  supportsReasoning: boolean;
};

export type ProviderModelCapabilityDeclaration = {
  capabilityPreset: ProviderCapabilityPresetId;
  capabilities: ProviderModelCapabilities;
};

export type ProviderModelUserConfig = {
  temperature: number;
  maxOutputTokens: number;
  reasoningEffort: ProviderReasoningEffort | null;
  reasoningBudgetTokens: number | null;
};

export type ProviderModelConfig = {
  id: string;
  name: string;
  model: string;
  capabilityPreset: ProviderModelCapabilityDeclaration["capabilityPreset"];
  capabilities: ProviderModelCapabilityDeclaration["capabilities"];
  temperature: ProviderModelUserConfig["temperature"];
  maxOutputTokens: ProviderModelUserConfig["maxOutputTokens"];
  reasoningEffort: ProviderModelUserConfig["reasoningEffort"];
  reasoningBudgetTokens: ProviderModelUserConfig["reasoningBudgetTokens"];
};

export type ProviderConfig = {
  id: string;
  name: string;
  protocol: ProviderProtocol;
  baseUrl: string;
  apiKeyEnvVar: string;
  apiKeyValue: string;
  apiKeyPresent: boolean;
  models: ProviderModelConfig[];
  selectedModelId: string | null;
};

export type ProviderRegistry = {
  providers: ProviderConfig[];
  selectedProviderId: string | null;
};

export type ApiKeyWriteResult = {
  providerId: string;
  envVarName: string;
  storedToUserScope: boolean;
};
