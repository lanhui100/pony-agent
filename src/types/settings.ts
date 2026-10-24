export type WorkspaceMode = "coding" | "work";

export type AppSettings = {
  workspaceMode: WorkspaceMode;
  /**
   * Wave43 D7：全局 Agent Trace 上报开关（落盘键 `agentTraceReporting`）。
   * 契约：缺省 `false`（默认关闭）；为 `false` 时 core 侧零 span 分配、零网络出口。
   */
  agentTraceReporting: boolean;
};
