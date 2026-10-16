# wave-22 契约冻结矩阵：并行 Tool Calls、读前写物理硬不变量与真实 OS 级沙箱隔离

## 1. 现状与诊断
- 工具调用串行化：`ProviderDecision` 限制 `tool_call: Option<ToolCall>`，Anthropic 等上游一旦并行吐出多个 `tool_use` 会硬报错打断轮次，模型无法并行并发读文件或执行搜索。
- 读前写仅靠 Prompt 软约束：`workspace_edit_file` 在未读文件时仍可随意写入，容易导致模型盲写与代码大面积踩踏破坏。
- 沙箱仅是判定门而非隔离墙：`NativeSandboxBackend` 在 Linux 仅探测 bwrap 是否在 PATH 中，命令执行时仍走裸 `Command::new`。

## 2. 目标契约 (C1 ~ C3)

### C1: 读前写物理硬不变量
- 在会话调度层维护已读文件路径集合（已观察文件观察簿 `observed_files`）。
- 任何 `workspace_edit_file` 调用在执行前必须校验目标文件已包含在观察簿中，否则直接返回 `fs_not_observed` 错误，引导模型先执行 `workspace_read_file`。

### C2: 并行 Tool Calls 接收与调度支持
- `ProviderDecision` 升级为支持 `pub tool_calls: Vec<ToolCall>`。
- 兼容既有单个 `tool_call` 快捷访问方法；流式解析器完整接收所有 index 上的 tool blocks。

### C3: 真实沙箱隔离执行
- Linux 环境下当 `bwrap` 探测可用时，命令执行包装入 `bwrap --ro-bind / / --dev /dev --bind <workspace> <workspace> ...`；在沙箱不可用时显式标记 `unsandboxed: true` 并告警。
