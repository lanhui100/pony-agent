# PA-054 实现 Phase D 桥接能力入口：MCP Resource / ToolSearch

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 依赖：
  [add-second-wave-tool-surface](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface>)

## Canonical Spec
- 依赖：
  [second-wave-tool-surface/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)

## 背景
`PA-050` 已定义第二批工具按 `Phase A -> Phase B -> Phase C -> Phase D` 顺序推进。当前：

- `Phase A` 的 `Edit / Write / Run` 已完成第一轮实现与验证
- `Phase B` 的 `Glob / Grep-like Search` 已完成第一轮实现与验证
- `Phase C` 的 `WebFetch / WebSearch` 已完成第一轮实现，正在补宿主层回归与成功路径验证

下一步应进入 `Phase D`，补齐与 Codex / Claude Code 基础工具面更接近的桥接能力入口：

- `MCP Resource`
- `ToolSearch`

当前仓库并不是从零开始。`PA-020` 已完成 capability bridge 第一版，代码中已经存在：

- `CapabilityKind::Resource`
- `CapabilityInvocationMode::ReadOnlyFetch`
- `register_mcp_resource_capability(...)`
- `resource_fetch_success_result(...)`
- `resource_fetch_failure_result(...)`
- `list_capabilities / inspect_capability`

也就是说，`Phase D` 的重点不是重造 bridge，而是在现有 capability registry / control plane 基础上，把“模型可用的最小入口工具”正式接出来。

## 目标
把 `Phase D` 的桥接能力以最小可运行入口落成真实 builtin tool surface，并保持它们与现有 workspace / web 工具边界清晰。

## 输出
- `MCP Resource` 的最小只读 builtin 入口
- `ToolSearch` 的最小 discovery builtin 入口
- 对应工具注册、canonical 映射、permission facts、前端默认工具目录和测试
- 对 `Phase D` 与现有 capability bridge 的衔接说明

## 范围边界
- 本卡只做 `Phase D` 最小入口
- 不重做 `PA-020` 已完成的 capability registry / snapshot / monitor bridge
- `MCP Resource` 只负责只读资源获取，不混入普通工具执行
- `ToolSearch` 只负责 deferred / dynamic discovery，不混入普通内容搜索
- 本卡不提前实现 Browser、Automation、Thread 管理或完整 marketplace/tool ranking
- 如果真实 connector 仍未接入，可以先基于现有 registry / snapshot 做最小可验证入口，但要保持合同可演进

## 验收标准
- `MCP Resource` 出现在真实 builtin tool surface 中
- `ToolSearch` 出现在真实 builtin tool surface 中，且被标记为 deferred / discovery 导向能力
- `MCP Resource` 能通过现有 capability bridge 读取 `CapabilityKind::Resource` 条目，并返回结构化结果或结构化失败
- `ToolSearch` 能基于现有 capability registry 返回结构化工具候选结果，而不是复用普通文本搜索输出
- Rust 测试、宿主层相关回归和前端相关测试通过

## 当前进展
- 已确认 `PA-020` 的 capability bridge 已具备 `resource` 基础合同，不需要重造桥
- 已确认 `second-wave-tool-surface` spec 中 `Phase D` 明确要求：
  - 至少考虑只读 `MCP Resource`
  - 将 `ToolSearch` 视为 deferred / dynamic tool discovery 能力
- 已完成 `MCP Resource / ToolSearch` 最小 builtin 入口实现，并完成：
  - capability mediation 接线
  - canonical / dotted alias 支持
  - `ToolSearch` 的 limit clamp、空结果、source 过滤与结构化候选结果
  - `MCP Resource` 的 `missing_capability_id` / not-found / structured result 回归
- 已完成 `opencode / deepseek-v4-flash` 实现审阅并采纳高收益问题，包括：
  - `blocked_tool_result` 结构化 JSON
  - registry 工具错误路径与空结果回归
- 已完成全量验证：
  - `npm run test:unit`
  - `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test`
  - `npm run cargo:test:regression`
  - `npm run test:tauri:smoke`
  - `npm run test:e2e`

## 下一步动作
1. 当前卡已完成；后续如需继续扩到 `mcp_resource_list`、prompt template 读面或更强 discovery 排序，再单独开新卡

## 当前卡点
- 暂无。当前卡已完成态收口。

## 断点续跑提示
继续前先看：

- [PA-050](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-050-build-second-wave-tool-surface.md>)
- [PA-053](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-053-implement-phase-c-external-reading.md>)
- [PA-020](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md>)
- [second-wave-tool-surface spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)
- [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/capability_bridge.rs)
- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
