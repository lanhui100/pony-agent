import type { RuntimePhase, ToolActivity } from "@/types/runtime";

export type AgentRuntimeMode =
  | "connecting"
  | "thinking"
  | "responding"
  | "tool_drafting"
  | "executing_tool"
  | "waiting_approval"
  | "retrying";

export interface AgentRuntimeStatus {
  mode: AgentRuntimeMode;
  label: string;
  detail?: string;
  toolName?: string;
  canonicalToolName?: string | null;
  startTime?: number | null;
  retryAttempt?: number;
  retryMax?: number;
}

export interface ResolveRuntimeStatusOptions {
  isSubmitting: boolean;
  phase: RuntimePhase;
  toolActivities?: ToolActivity[];
  turnStartedAt?: number | null;
  streamingReasoning?: boolean;
  hasStreamingContent?: boolean;
  retryAttempt?: number;
  retryMax?: number;
}

export function resolveAgentRuntimeStatus(
  options: ResolveRuntimeStatusOptions
): AgentRuntimeStatus | null {
  const {
    isSubmitting,
    phase,
    toolActivities = [],
    turnStartedAt,
    streamingReasoning,
    hasStreamingContent,
    retryAttempt,
    retryMax,
  } = options;

  if (!isSubmitting) {
    return null;
  }

  // 0. 重试状态投射
  if (typeof retryAttempt === "number" && retryAttempt > 0) {
    const max = retryMax || 3;
    return {
      mode: "retrying",
      label: `正在重试（${retryAttempt}/${max}）...`,
      startTime: turnStartedAt,
      retryAttempt,
      retryMax: max,
    };
  }

  // 1. 优先检查等待用户授权
  if (phase === "waiting_user") {
    return {
      mode: "waiting_approval",
      label: "等待确认授权...",
      startTime: turnStartedAt,
    };
  }

  // 2. 检查是否有正在执行的工具
  const runningTool = toolActivities.find((t) => t.status === "running");
  if (runningTool) {
    const name = runningTool.canonicalToolName || runningTool.name;
    const displayName = runningTool.displayNameZh || name;
    const label = displayName ? `执行工具: ${displayName}...` : "正在执行工具...";
    return {
      mode: "executing_tool",
      label,
      toolName: runningTool.name,
      canonicalToolName: runningTool.canonicalToolName ?? null,
      detail: runningTool.description,
      startTime: turnStartedAt,
    };
  }

  // 3. 检查是否有计划中/生成中的工具调用（模型下发 tool_calls 参数中，或 planned 状态）
  const plannedTool = toolActivities.find((t) => t.status === "planned");
  if (plannedTool) {
    const name = plannedTool.canonicalToolName || plannedTool.name;
    const displayName = plannedTool.displayNameZh || name;
    const label = displayName ? `准备调用工具: ${displayName}...` : "准备调用工具...";
    return {
      mode: "tool_drafting",
      label,
      toolName: plannedTool.name,
      canonicalToolName: plannedTool.canonicalToolName ?? null,
      detail: plannedTool.description,
      startTime: turnStartedAt,
    };
  }

  // 4. calling_tool 阶段兜底（无 toolActivities 时）
  if (phase === "calling_tool") {
    return {
      mode: "executing_tool",
      label: "正在执行工具...",
      startTime: turnStartedAt,
    };
  }

  // 3. 模型思考阶段（流式 reasoning 输出中）
  if (streamingReasoning) {
    return {
      mode: "thinking",
      label: "思考中...",
      startTime: turnStartedAt,
    };
  }

  // 4. 模型回复输出阶段
  if (phase === "calling_model") {
    if (hasStreamingContent === false) {
      return {
        mode: "thinking",
        label: "思考中...",
        startTime: turnStartedAt,
      };
    }
    return {
      mode: "responding",
      label: "正在回复...",
      startTime: turnStartedAt,
    };
  }

  // 5. 连接与初始化阶段
  if (phase === "connecting") {
    return {
      mode: "connecting",
      label: "准备就绪中...",
      startTime: turnStartedAt,
    };
  }

  // 默认活跃中
  return {
    mode: "responding",
    label: "正在处理...",
    startTime: turnStartedAt,
  };
}

export function formatElapsedSeconds(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return `${minutes}m ${remainingSeconds}s`;
}
