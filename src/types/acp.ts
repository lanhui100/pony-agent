// ACP（Agent Client Protocol）宿主面类型（B163）。
//
// 真源：
// - 后端清单结构 `crates/pony-agent-core/src/acp/config.rs` 的 `AcpAgentSpec`（`camelCase`）；
// - 后端摘要结构 `crates/pony-agent-core/src/acp/host.rs` 的 `AcpAgentSummary`；
// - 命令返回 `src-tauri/src/lib.rs` 的 `AcpDelegateOutcome`；
// - 活跃委派快照 `crates/pony-agent-core/src/acp/registry.rs` 的 `ActiveDelegate`。
//
// 纪律：本文件只描述**既有**线缆形状，不新增前端侧推导字段——任何"界面算出来的值"都放
// 组件/store，不放协议类型里（否则后端改字段时前端会静默说谎）。

/** 清单里的一条 env 条目。`value` 只在本机使用，前端不应渲染（后端摘要面只回键名）。 */
export type AcpEnvVariable = {
  name: string;
  value: string;
};

/** 宿主能力提示：`false` = 整块能力不下放（外部 agent 只能在自己的子进程里做）。 */
export type AcpCapabilitiesHint = {
  fs: boolean;
  terminal: boolean;
};

/** `session/new` 的 `mcpServers` 三态在此只表达"是否显式给定"；条目结构由后端 wire 类型决定。 */
export type AcpAgentSpec = {
  /** 稳定标识（`acp_delegate` 的 `agent` 参数按此解析），如 `opencode`。 */
  id: string;
  /** UI 展示名。 */
  label: string;
  /** 可执行文件名或绝对路径（按 `PATH` 解析）。 */
  command: string;
  args: string[];
  env: AcpEnvVariable[];
  /** 默认工作目录（后端保证是绝对路径）。 */
  defaultCwd?: string | null;
  capabilitiesHint: AcpCapabilitiesHint;
  /** 缺省 = inherit（复用 Pony 既有 MCP 配置）；`[]` = 显式禁用；条目 = 仅用条目。 */
  mcpServers?: unknown[] | null;
  /** 是否继承 `~/.pony/proxy.env` 的 `*_proxy`（缺省 true）。 */
  inheritProxyEnv: boolean;
};

/** `acp_list_agents` 的返回项：**不含 env 值**（只回键名，值永不出宿主）。 */
export type AcpAgentSummary = {
  id: string;
  label: string;
  command: string;
  args: string[];
  defaultCwd: string | null;
  /** 是否为内置默认项（后端 `config::is_builtin_agent` 单一真源）。 */
  builtin: boolean;
  /** 清单声明的 env **键名**。 */
  envKeys: string[];
  capabilities: AcpCapabilitiesHint;
};

/**
 * `acp_delegate_start` 的返回。
 *
 * `output` 是 `acp_delegate` 工具面的 JSON 原文（`ok` / `error.code` / `stopReason` 同源），
 * 与模型侧看到的回执**同一份契约**——前端不要另造错误码表。
 */
export type AcpDelegateOutcome = {
  /** 宿主侧分配的委派 id（取消时复用同一个）。 */
  delegationId: string;
  /** `ok` | `aborted` | `error`（与工具面 `ToolResult.status` 同一词表）。 */
  status: "ok" | "aborted" | "error" | string;
  output: string;
  durationMs: number;
};

/** 一次活跃委派快照（`acp::registry`，TUI Agents 页签与桌面端共用的只读面）。 */
export type AcpSessionView = {
  /** 进程内单调编号（登记顺序，界面用它稳定排序）。 */
  delegateId: number;
  /** 外部 agent 的稳定标识（`AcpAgentSpec.id`）。 */
  agentId: string;
  /** ACP 协议层的会话 id（`session/new` 回执）。 */
  acpSessionId: string;
  /** 发起委派的宿主会话；`null` = 无会话上下文，不归任何会话。 */
  hostSessionId: string | null;
  cwd: string;
  startedAtMs: number;
};

/**
 * `acp://delegate` 事件的负载（后端 `AcpDelegateEventPayload`）。
 *
 * `name` 沿用回合事件词表（`turn:delta` / `turn:tool` …）：委派期间的事件名与回合内同形，
 * 因此必须靠 `delegationId` 归属，不能直接塞进当前回合时间线。
 */
export type AcpDelegateEvent = {
  delegationId: string;
  name: string;
  payload: unknown;
};
