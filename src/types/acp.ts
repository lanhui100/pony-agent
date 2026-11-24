/**
 * ACP 外部 agent 清单（前端只读投影）。
 *
 * 真源：`crates/pony-agent-core/src/acp/config.rs` 的 `AcpAgentSpec`（`serde(rename_all = "camelCase")`）。
 * 只投影 UI 实际消费的字段：`mcpServers` 的 stdio/http/sse 三变体属于清单**写面**细节，
 * P2 只读面不消费，故不在此建模——避免同一结构在前端造第二份真源。
 */
export interface AcpCapabilitiesHint {
  /** 是否下放 `clientCapabilities.fs.{readTextFile,writeTextFile}`。 */
  fs: boolean;
  /** 是否下放 `clientCapabilities.terminal`。 */
  terminal: boolean;
}

/** 清单里的环境变量条目（宿主注入仍受 env 白名单约束）。 */
export interface AcpEnvVariable {
  name: string;
  value: string;
}

/** 一条外部 ACP agent 的启动规格（用户可增删的清单条目）。 */
export interface AcpAgentSpec {
  /** 稳定标识（`acp_delegate` 的 `agent` 参数按此解析），如 `opencode`。 */
  id: string;
  /** UI 展示名。 */
  label: string;
  /** 可执行文件名或绝对路径（按 `PATH` 解析）。 */
  command: string;
  /** 传给命令的参数。 */
  args: string[];
  /** 额外环境变量。 */
  env: AcpEnvVariable[];
  /** 默认工作目录（绝对路径）；缺省时由宿主派生。 */
  defaultCwd?: string | null;
  /** 宿主能力提示（fail-closed 的输入）。 */
  capabilitiesHint: AcpCapabilitiesHint;
  /** `session/new` 的 `mcpServers` 三态；只读面不消费其内部结构。 */
  mcpServers?: unknown[] | null;
  /** 是否继承 `<HOME>/.pony/proxy.env` 的 `*_proxy` 变量。 */
  inheritProxyEnv: boolean;
}
