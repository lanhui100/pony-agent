export type ProviderProtocol = "openai" | "anthropic";

export type ProviderAuthType = "auto" | "bearer" | "x-api-key";

export type ProviderReasoningEffort = "low" | "medium" | "high" | "max";

export type ProviderCapabilityPresetId =
  | "auto"
  | "open-ai-chat"
  | "open-ai-reasoning"
  | "anthropic-thinking"
  | "deepseek-chat"
  | "deepseek-reasoner"
  | "custom";

export type ProviderProtocolEndpoint = {
  protocol: ProviderProtocol;
  enabled: boolean;
  baseUrl: string;
  authType: ProviderAuthType;
};

export type ProviderModelCapabilities = {
  contextWindowTokens: number | null;
  supportsTools?: boolean;
  supportsStreaming?: boolean;
  supportsReasoning: boolean;
  supportsImageInput: boolean;
  supportsVideoInput: boolean;
  supportsAudioInput: boolean;
  supportsTextOutput: boolean;
  supportsImageOutput: boolean;
  supportsVideoOutput: boolean;
  supportsAudioOutput: boolean;
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
  protocol?: ProviderProtocol | null;
};

export type ProviderModelConfig = ProviderModelIdentity &
  ProviderModelCapabilityDeclaration &
  ProviderModelUserPolicy;

export type ProviderConfig = {
  id: string;
  name: string;
  protocol: ProviderProtocol;
  baseUrl: string;
  authType: ProviderAuthType;
  supportedProtocols: ProviderProtocol[];
  endpoints: ProviderProtocolEndpoint[];
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
