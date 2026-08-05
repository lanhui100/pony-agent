## Why

Pony Agent 已具备基础 coding-agent 工具面，但当前产品工具、执行原语、权限事实与组合执行之间仍存在可导致治理旁路和协议误导的偏差：`Plan` 实际等同任意批量执行、`Ask` 仍是回显占位、组合子步骤绕过统一 capability mediation，且 `Run`、`WebFetch`、`Search` 的资源与安全边界不足。继续叠加 LSP、子代理或 workflow 会放大这些问题，因此现在必须先加固工具运行内核，再按依赖顺序扩展能力。

## What Changes

- 将工具定义、模型暴露、执行注册和权限声明收敛为单一描述符来源，取消依赖名称表和工具数组长度猜测 builtin 身份的逻辑。
- 引入受治理的 tool dispatcher；所有 builtin、capability、skill 和 composite 子调用统一经过调用来源检查、schema 校验、可改写 hook、最终参数鉴权、取消、预算、遥测与结果归一化。
- 将 `Plan` 从 `workspace_batch` 中拆出为控制面工具，将 `Ask` 从 `echo_input` 中拆出为真实 host-mediated 澄清边界。
- 修复组合工具权限聚合，保留每个子步骤的 scope、审批、决策来源和执行证据。
- 把 `Run` 重构为有生命周期的进程工具族，提供启动/轮询/输入/终止能力，并要求无人值守执行始终处于真实 sandbox；没有 sandbox backend 时 fail closed。
- 加固 `WebFetch` 的 URL、重定向、SSRF、响应体、内容类型和下载预算边界；任意 URL 抓取在 pinned connector、逐跳 peer-IP 校验与环境代理隔离完成前保持禁用。
- 用标准 regex/glob/ignore 语义替换伪 regex 和不稳定的前 800 文件遍历，并显式报告扫描截断。
- 补齐 `view_image` 与基于真实 `McpTransport` 的 MCP resource list/template/read 基础能力；完成 deferred `ToolSearch` 的同一 turn 下一 provider hop 提升闭环。
- 将 LSP、任务/Goal、worktree、子代理和 workflow 作为依赖于新 dispatcher 的后续阶段，不在本 change 中直接实现。
- 由任务卡 `PA-076` 跟踪实现、审核、验证和断点续作。

## Capabilities

### New Capabilities

- `tool-runtime-dispatch`: 统一工具描述符、注册、分发、组合子调用治理、预算、取消和动态暴露边界。
- `process-tool-lifecycle`: 受控进程启动、轮询、stdin、终止、输出预算和进程树清理合同。
- `web-access-safety`: URL 抓取的网络范围、重定向、SSRF、响应体和内容类型安全合同。

### Modified Capabilities

- `tool-system-contract`: 将当前派生式工具元数据收口为单一描述符真相源，并要求所有执行来源走统一 dispatcher。
- `tool-permission-contract`: 将权限决策落实到 builtin 与 composite 的每个实际子调用，修复弱化和绕过。
- `third-wave-default-tool-alignment`: 把 `Plan` 与 `Ask` 从兼容占位映射升级为真实控制面能力，并补齐 deferred 工具提升行为。
- `second-wave-tool-surface`: 收紧 `Run`、`Search`、`Glob`、`WebFetch` 与 MCP resource 的实现级安全和完整性要求。
- `tool-observability-contract`: 增加组合子步骤、动态工具提升、进程生命周期和资源截断的统一观测要求。

## Impact

- Core：`crates/pony-agent-core/src/agent/tools.rs` 将拆分为描述符、registry/dispatcher、基础原语、控制工具和组合工具模块；runtime、provider、planner、capability bridge 与 telemetry 将改为消费统一注册表。
- Host：`src-tauri` 需要实现真实用户提问、审批、图片读取与进程生命周期的宿主中介接口。
- Frontend：需要消费待回答、待审批、进程运行和动态工具提升状态，但不重新推导权限真相。
- Dependencies：预计新增或启用 `regex`、`globset`、`ignore`、URL/IP 校验和平台进程树管理相关依赖；具体选择在 design 中固定。
- Compatibility：保留现有产品级工具名；旧 `workspace_*` 名称仅作内部原语或有界兼容别名。现有 `Plan -> workspace_batch`、`Ask -> echo_input` 行为将被替换，属于预期的行为修正。
