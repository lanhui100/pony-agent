import type { RuntimePhase, ToolActivity } from "@/types/runtime";

export type AgentRuntimeMode =
  | "connecting"
  | "thinking"
  | "responding"
  | "executing_tool"
  | "waiting_approval";

export interface AgentRuntimeStatus {
  mode: AgentRuntimeMode;
  label: string;
  detail?: string;
  toolName?: string;
  startTime?: number | null;
}

export interface ResolveRuntimeStatusOptions {
  isSubmitting: boolean;
  phase: RuntimePhase;
  toolActivities?: ToolActivity[];
  turnStartedAt?: number | null;
  streamingReasoning?: boolean;
}

export function resolveAgentRuntimeStatus(
  options: ResolveRuntimeStatusOptions
): AgentRuntimeStatus | null {
  const { isSubmitting, phase, toolActivities = [], turnStartedAt, streamingReasoning } = options;

  if (!isSubmitting) {
    return null;
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
  if (runningTool || phase === "calling_tool") {
    const name = runningTool?.canonicalToolName || runningTool?.name || "tool";
    const label = `执行工具: ${runningTool?.displayNameZh || name}...`;
    return {
      mode: "executing_tool",
      label,
      toolName: runningTool?.name,
      detail: runningTool?.description,
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
