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
