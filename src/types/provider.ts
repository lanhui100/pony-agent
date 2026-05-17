export type ProviderProtocol = "openai" | "anthropic";

export type ProviderModelConfig = {
  id: string;
  name: string;
  model: string;
  temperature: number;
  maxOutputTokens: number;
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
