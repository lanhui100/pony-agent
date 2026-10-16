# wave-21 契约冻结矩阵：工作区规则热加载与标准 Stdio MCP 传输

## 1. 现状与诊断
- 死代码隔离：`workspace_hotloader.rs` 虽具备完善的 AGENTS.md/CLAUDE.md 与 SKILL.md 动态哈希重载和前缀构造能力，但在实际轮次构建路径中未被调用；模型在未被告知项目规则的情况下工作。
- MCP 客户端缺位：`McpTransport` trait 在全仓仅有 `FakeMcpTransport` 测试双重，缺少连接外部实际子进程的 `StdioMcpTransport`。

## 2. 目标契约 (C1 ~ C2)

### C1: 动态规则热加载注入层叠 Prompt
- 在 `DefaultTurnContextBuilder` 生成 `ProviderRequest` 的过程中，基于传入的会话与工作区目录调用 `WorkspaceHotloader`；
- 当工作区存在 `AGENTS.md` 时，其规则文本作为 `project_instruction_messages` 或前缀注入，且当 `AGENTS.md` 内容变更时能感知变化。

### C2: StdioMcpTransport 实现
- 实现 `StdioMcpTransport` 结构体，支持配置 `command: String`, `args: Vec<String>`, `env: HashMap<String, String>`；
- 实现 `McpTransport::execute(&self, request: &McpTransportRequest) -> Result<McpTransportResponse, String>`：
  - 基于 JSON-RPC 2.0 格式构造并在执行时通过管道与子进程交互，读取应答并封装为 `McpTransportResponse`。
