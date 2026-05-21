export type ProviderProtocol = "openai" | "anthropic";

export type ProviderReasoningEffort = "minimal" | "low" | "medium" | "high";

export type ProviderModelCapabilities = {
  contextWindowTokens: number | null;
  supportsTools: boolean;
  supportsStreaming: boolean;
  supportsImageInput: boolean;
  supportsReasoning: boolean;
};

export type ProviderModelConfig = {
  id: string;
  name: string;
  model: string;
  temperature: number;
  maxOutputTokens: number;
  reasoningEffort: ProviderReasoningEffort | null;
  reasoningBudgetTokens: number | null;
  capabilities: ProviderModelCapabilities;
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
