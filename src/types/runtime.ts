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
  | "calling_model"
  | "calling_tool"
  | "failed";

export type ToolActivity = {
  id: string;
  name: string;
  status: "planned" | "running" | "done" | "error";
  summary: string;
};

export type TraceStep = {
  id: string;
  label: string;
  state: "completed" | "active" | "pending" | "error";
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  status?: "pending" | "done" | "error";
  modelName?: string | null;
};

export type TurnInput = {
  message: string;
  providerId?: string | null;
  modelId?: string | null;
  history?: TurnHistoryMessage[];
};

export type TurnHistoryMessage = {
  role: "user" | "assistant";
  content: string;
};

export type TurnResult = {
  phase: RuntimePhase;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
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
