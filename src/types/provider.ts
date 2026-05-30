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

export type ProviderModelUserPolicy = {
  temperature: number;
  maxOutputTokens: number;
  reasoningEffort: ProviderReasoningEffort | null;
  reasoningBudgetTokens: number | null;
};

export type ProviderModelUserConfig = ProviderModelUserPolicy;

export type ProviderModelIdentity = {
  id: string;
  name: string;
  model: string;
};

export type ProviderModelConfig = ProviderModelIdentity &
  ProviderModelCapabilityDeclaration &
  ProviderModelUserPolicy;

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
