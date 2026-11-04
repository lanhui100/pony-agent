import type { RuntimePhase, ToolActivity } from "@/types/runtime";

export type AgentRuntimeMode =
  | "connecting"
  | "thinking"
  | "responding"
  | "tool_drafting"
  | "executing_tool"
  | "waiting_approval"
  | "retrying"
  /** wave-60 §C7：停止请求已受理、终态未到（进行中态，仅流式期可见）。 */
  | "stopping"
  /**
   * wave-64 §2 F1：停止的**终态**（`turn:cancelled` 后由 store 按 phase 派生）。
   * 与 `stopping`（进行中）严格区分：中性 stone 语义色、lucide `Pause`、无 shimmer、无耗时计时。
   */
  | "stopped"
  | "error"
  /** C-9：中性/成功反馈（如斜杠命令提示），非错误、非进行中。 */
  | "info";

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

  // wave-60 §C7 / 补遗 v2 A2：停止请求受理后优先呈现「正在停止…」。
  // 口径：仅在 `isSubmitting`（流式进行中）时返回 —— 终态后本函数恒返回 null，
  // 终态「已停止」由 `stores/runtime.ts` 的 `agentRuntimeStatus` getter 以
  // `mode === "stopped"` 派生承载（wave-64 §2 F1，不再落 `sessionNotice`），二者不互相冒充。
  if (phase === "stopping") {
    return {
      mode: "stopping",
      label: "正在停止…",
      startTime: turnStartedAt,
    };
  }

  // 0. 重试状态投射
  // Wave-59 F4-5（RC9）：本分支由 `stores/runtime.ts` 的 agentRuntimeStatus getter 真实接线
  // （超时重试提交时 `TIMEOUT_RETRY_PENDING_MESSAGE` 占位 → retryAttempt=1），不再是不可达分支。
  // 缺省 retryMax 时降级为「正在重试…」：无后端「最大重试次数」信号，不得虚构分母冒充能力。
  if (typeof retryAttempt === "number" && retryAttempt > 0) {
    const max = typeof retryMax === "number" && retryMax > 0 ? retryMax : null;
    return {
      mode: "retrying",
      label: max != null ? `正在重试（${retryAttempt}/${max}）...` : "正在重试…",
      startTime: turnStartedAt,
      retryAttempt,
      retryMax: max ?? undefined,
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
    if (streamingReasoning || hasStreamingContent === false) {
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
    return `${seconds}秒`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return remainingSeconds > 0 ? `${minutes}分${remainingSeconds}秒` : `${minutes}分钟`;
}
