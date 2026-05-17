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
  status: "planned" | "running" | "done";
  summary: string;
};

export type TraceStep = {
  id: string;
  label: string;
  state: "completed" | "active" | "pending";
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
};

export type TurnInput = {
  message: string;
  providerId?: string | null;
  modelId?: string | null;
};

export type TurnResult = {
  phase: RuntimePhase;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerMode: string;
  fallbackReason?: string | null;
  userMessage: string;
  assistantMessage: string;
  traceSteps: TraceStep[];
  toolActivities: ToolActivity[];
  sessionSummary: string;
};
