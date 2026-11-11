# PA-020 建立 MCP capability bridge

## 状态
- Status: `Done`
- Priority: `P3`
- Owner: `Codex`

## OpenSpec Change
- [add-mcp-capability-bridge](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-mcp-capability-bridge)

## Delta Spec
- [mcp-capability-bridge/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-mcp-capability-bridge/specs/mcp-capability-bridge/spec.md)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
在 core runtime、graph run、host control plane 与 memory/planner 边界稳定之后，把 MCP 作为能力接入层正式引入，让外部资源、工具与上下文可以通过统一 bridge 接进 Pony Agent，而不是直接侵入 runtime / graph 内部。

## 输出
- MCP bridge 第一版抽象
- MCP resource / tool / prompt-like capability 映射规则
- MCP 与内建 tools / skills / memory retrieval 的边界说明
- capability registry 中的 MCP 条目模型
- MCP 错误、权限与可观测性约束

## 验收标准
- MCP 被定义为能力接入层，而不是 graph 或 runtime 的调度层
- 内建 tools 与 MCP tools 可以被统一 capability registry 暴露
- planner / graph 只消费抽象 capability facts，不直接耦合 MCP 协议细节
- 宿主层不需要自己理解 MCP，只走统一 control plane / capability query
- 文档明确本卡不要求一次性做完所有外部协议或 marketplace 生态

## 当前进展
- 已补齐 `OpenSpec proposal / design / tasks / delta spec`，变更名为 `add-mcp-capability-bridge`
- 后端已新增 `capability_bridge` 模块，先把 builtin tools 规范化为统一 `CapabilitySourceView / CapabilityView`
- `HostControlPlane` 与 Tauri command 已补 `list_capability_sources / list_capabilities / inspect_capability / inspect_capability_source`
- runtime 已新增 capability bridge 执行入口，当前 builtin tool call 会先 resolve 到 normalized capability，再进入既有 tool executor
- capability bridge 已支持 `tool / resource / prompt_template` 三类 action resolve 合同，并补上 `permissionScope`
- 前端 `runtime` 类型与 store 已补 capability read-plane 最小接线，并通过定向单测
- `ModelMonitorPage` 已接入 capability source / capability / inspect 的最小只读调试区块，避免前端继续停留在“有 store、无消费”
- Rust `cargo check --manifest-path src-tauri/Cargo.toml --lib` 已通过，当前 warning 主要是既有未使用项与 Windows incremental finalize 告警

## 下一步动作
- 把 MCP source / capability 注册接到真实来源，而不是仅 builtin fallback
- 在 runtime 下方补 capability bridge action，把执行入口从 `ToolCall` 直达逐步收敛到 bridge resolve
- 明确资源读取 / prompt template 展开与 tool execute 的结果、权限与失败归一化合同

## 当前卡点
- 如果在 planner、memory、control plane 未稳定前直接接 MCP，很容易让协议细节向上污染 graph、宿主和 UI

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-004-define-provider-and-tool-abstractions.md`
- `management/task-system/03_TASKS/PA-015-extract-host-control-plane.md`
- `management/task-system/03_TASKS/PA-019-build-graph-planner-and-decision-policy.md`
- `docs/architecture/overview.md`
- `openspec/changes/add-mcp-capability-bridge/`

## 2026-06-02 Update
- 已补 `HostControlPlane` 内部的 MCP source snapshot 写面，语义是“按 source 原子替换”，不对前端暴露注册命令。
- 已保证同一份 normalized snapshot 同步进入 control-plane read registry 与 runtime execution registry，避免双 registry 漂移。
- 已补运行时 resolve 规则：builtin alias 优先，其次按 tool label 匹配 host 注册的 MCP tool capability。
- 已补 Rust 定向验证：同 source 替换清理 stale capability、unreachable source 仍可 inspect、host 注册后 runtime 能 resolve MCP tool。

## Next Focus
- 接真实 discovery/connector，把 `McpSourceSnapshot` 从手工注入替换为生产来源。
- 继续补 `resource` fetch / `prompt_template` expansion 的统一执行结果合同。
- 再推进 planner consumption 与 observability 的正式验收项。

## 2026-06-02 Update 2
- 已修正 runtime tool 主路径的 failure 透传：`SourceUnavailable / PermissionDenied / MalformedResponse` 不再被吞成 `CapabilityNotFound`。
- 已把 capability 活动元数据挂入现有 `TurnToolActivity` 链路，并持久化到 session trace，可在 monitor drilldown 读取 `capabilityId / sourceId / invocationMode / failureKind / permissionScope`。
- 前端 `ModelMonitorPage` 已新增 trace 级 capability activity 展示，便于核对实际 source usage 与 failure class。
- 当前仍未完成的主缺口：`resource / prompt_template` 统一结果合同、capability 维度 summary 聚合、planner boundary 验收测试。
## 2026-06-02 Update 3
- 已把 capability 聚合读面接入 `ModelMonitorSummaryView`，新增 `capabilitySources / capabilityInvocationModes / capabilityFailureClasses` 三组 summary rows。
- `ModelMonitorPage` 已新增 capability summary 区块，可直接查看 source usage、invocation type 和 failure class，不再只能通过 trace drilldown 排查。
- 已补 planner/runtime boundary 验收测试，确认 planner 只保留 normalized `ToolCall { name, arguments }`，MCP resolve 仍发生在 runtime capability bridge 内部。
- `openspec/changes/add-mcp-capability-bridge/tasks.md` 已勾选 `5.2 / 6.1 / 6.3`，当前主剩余缺口是 `3.4` 和 `4.3`。

## Next Focus 2
- 补 `3.4`，为 `resource` fetch / `prompt_template` expansion 加上统一 result/failure contract。
- 收口 `4.3`，把 planner consumption rule 明确回写到 spec/design，与新增 acceptance test 对应。
## 2026-06-02 Update 4
- 已补 `3.4` 的 normalized `resource` fetch / `prompt_template` expansion result and failure contracts，并追加定向 Rust 单测。
- `proposal.md / design.md / spec.md / tasks.md` 当前已经覆盖 capability-ingress 定位、依赖边界、non-goals 与 planner consumption rule，`tasks.md` 已全部勾选完成。
- 本卡已完成；上文中的 `Next Focus`、`当前未完成主缺口` 等段落均为历史推进记录，不再代表当前状态。
- 已补 runtime-generated capability activity 的端到端验收测试，并确认前端 `npm run build` 通过。
- 稳态 spec 已落到 `openspec/specs/mcp-capability-bridge/spec.md`，change 已归档到 `openspec/changes/archive/2026-06-02-add-mcp-capability-bridge/`。
