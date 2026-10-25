export type WorkspaceMode = "coding" | "work";

export type AppSettings = {
  workspaceMode: WorkspaceMode;
  /**
   * Wave43 D7：全局 Agent Trace 上报开关（落盘键 `agentTraceReporting`）。
   * 契约：缺省 `false`（默认关闭）；为 `false` 时 core 侧零 span 分配、零网络出口。
   */
  agentTraceReporting: boolean;
  /** Wave-4 C1：OTel 端点与鉴权配置（支持 settings 持久化 > 进程 env > 默认内置端点） */
  otelEndpoint?: string;
  otelToken?: string;
  otelServiceName?: string;
  otelSamplingRatio?: number;
  /** Wave-5 K3：会话成本预算上限 (USD)。<= 0 或未设置时视为无预算限制。 */
  costBudgetUsd?: number | null;
  /** Wave-7 G2：用量治理策略配置（与 Rust serde camelCase 严格一致） */
  usageGovernancePolicy?: {
    window?: "session" | "day";
    softRatio?: number | null;
    hardLimitUsd?: number | null;
    enforcement?: boolean;
  };
};
