// 协议规范名（wire 值，与后端 ProviderProtocol serde rename 一致）。
// 反序列化兼容旧值：openai → openai-completions、anthropic → anthropic-messages，
// 未知值回落 openai-completions（见 stores/providers.ts 的 normalizeLegacyProtocol）。
export type ProviderProtocol = "openai-responses" | "openai-completions" | "anthropic-messages";

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
  /** 模型级 Base URL 覆盖；空串/空白在规范化时折叠为 null（= 继承提供商解析值）。 */
  baseUrl?: string | null;
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
  /**
   * PA-172 §2.2：上游调用策略（camelCase 与 Rust `ProviderRegistryView.upstreamPolicy` 对齐）。
   *
   * 可选：缺省 ⇒ 后端回落 45000/5/30000（G2 默认零行为变更）。声明在此是为了让
   * 「读入 → 修改 → 保存」整链不丢字段（否则前端保存会把策略静默清空）。
   */
  upstreamPolicy?: {
    requestTimeoutMs: number;
    decisionMaxAttempts: number;
    retryBudgetMs: number;
  };
  /**
   * PA-172 §2.2：跨 provider 故障转移候选链（provider id 列表，保序）。
   *
   * 可选：缺省/空数组 ⇒ 关闭换手（G2）。链长上限 8，卫生（未知/重复/自身）由后端解析时判定。
   */
  failoverProviderIds?: string[];
};

export type ApiKeyWriteResult = {
  providerId: string;
  envVarName: string;
  storedToUserScope: boolean;
};
