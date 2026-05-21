export type HealthPayload = {
  appName: string;
  appVersion: string;
  runtime: string;
  graphEngine: string;
};

export type RuntimePhase =
  | "idle"
  | "connecting"
  | "ready"
  | "completed"
  | "calling_model"
  | "calling_tool"
  | "failed";

export type ToolActivity = {
  id: string;
  name: string;
  status: "planned" | "running" | "done" | "error";
  summary: string;
  argumentsText?: string | null;
  resultText?: string | null;
  durationSeconds?: number | null;
};

export type AvailableTool = {
  name: string;
  description: string;
  inputSchema: {
    type?: string;
    properties?: Record<string, { type?: string; description?: string }>;
    required?: string[];
    additionalProperties?: boolean;
  };
};

export type TraceStep = {
  id: string;
  label: string;
  state: "completed" | "active" | "pending" | "error";
};

export type ChatMessage = {
  id: string;
  turnId: string;
  role: "user" | "assistant" | "tool";
  content: string;
  status?: "pending" | "done" | "error";
  modelName?: string | null;
  tokenCount?: number | null;
  toolName?: string | null;
  detail?: string | null;
  durationSeconds?: number | null;
};

export type TurnTraceRecord = {
  turnId: string;
  title: string;
  phase: RuntimePhase;
  traceSteps: TraceStep[];
  toolActivities: ToolActivity[];
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  sessionSummary?: string | null;
  fallbackReason?: string | null;
  error?: string | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  updatedAt: number;
};

export type TurnInput = {
  message: string;
  providerId?: string | null;
  modelId?: string | null;
  sessionId?: string | null;
  history?: TurnHistoryMessage[];
};

export type TurnHistoryMessage = {
  role: "user" | "assistant";
  content: string;
};

export type SessionOverview = {
  conversationId: string;
  title?: string | null;
  summary: string;
  turnCount: number;
  lastReferencedFile?: string | null;
  updatedAtMs: number;
};

export type SessionSnapshot = {
  conversationId: string;
  title?: string | null;
  summary: string;
  history: TurnHistoryMessage[];
  turnCount: number;
  lastReferencedFile?: string | null;
  updatedAtMs: number;
};

export type TurnResult = {
  phase: RuntimePhase;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason?: string | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  userMessage: string;
  assistantMessage: string;
  traceSteps: TraceStep[];
  toolActivities: ToolActivity[];
  sessionSummary: string;
};

export type TurnStreamEvent = {
  turnId: string;
  kind: "started" | "delta" | "trace" | "tool" | "completed" | "failed";
  phase?: RuntimePhase | string | null;
  text?: string | null;
  error?: string | null;
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  fallbackReason?: string | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  traceSteps?: TraceStep[] | null;
  toolActivities?: ToolActivity[] | null;
  sessionSummary?: string | null;
};
