# 2026-07-18 PA-076 Spec Review and Registry Foundation

## 本轮目标

- 继续 `PA-076 / harden-and-expand-agent-tool-runtime`
- 结束 C 级规格审核门禁，落入 P0/P1 修订
- 开始不会触碰用户 runtime trace 改动的 descriptor/registry 基础层

## 本轮完成

- 完成架构、安全、测试三路独立审核及 consultant 裁决。
- 创建审核记录：`02_REVIEWS/2026-07-18-pa076-spec-review.md`。
- 将以下安全合同写入 proposal/design/tasks/delta specs：
  - hook 改写后最终参数重新校验、鉴权与 sandbox 决策
  - `ToolOutcome.execution_status` 与 `control_outcome` 正交
  - 带 session/run/turn/call、descriptor snapshot、args/policy digest、nonce/version/expiry 的 `PendingControlRequest`
  - model/child/host invocation origin 与 `TurnToolView`
  - autonomous Run 的 real sandbox fail-closed
  - arbitrary URL WebFetch 的 pinned connector/peer-IP/ambient proxy fail-closed
  - source-bound `McpTransport` 与独立 `ResourceTemplate`
- 新增 migration-aware characterization tests：产品工具面、alias、provider payload、permission projection、legacy batch write child；Ask/Plan placeholder 映射被明确标为迁移删除基线。
- 新增 `ToolDescriptor / ToolRegistrySnapshot`，provider 已不再以工具数组长度推断 builtin；builtin capability 投影现在读取 descriptor permission facts。
- 新增独立 `tool_runtime` contract module：`InvocationOrigin`、`PendingControlRequest`、`TurnToolView`、`SandboxBackend`、`McpTransport`。

## 验证

- `npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict`：通过。
- `git diff --check`：通过。
- `rg`：provider 生产逻辑不存在基于 builtin tool count 的推断；唯一 count 使用位于新回归测试的诱饵构造。
- `rustfmt --edition 2021`：对本轮 Rust 文件及 regression test 完成。
- `npm run cargo:test:exact -- --lib agent::tools::contract_view_tests::`：未通过执行环境，依赖编译阶段报 `link.exe not found`。未将此记录为测试通过。

## 当前状态

- `tasks.md` 的 1.1、1.2、1.3 已完成；第 2 阶段进行中。
- 2026-07-19 已完成 2.2、2.3、2.4、2.5、2.7 的合同实现：typed permission declaration、registry namespace/provenance/cycle/source-replacement 校验、`TurnToolView` provider projection，以及 process/resolver/MCP/clock fake harness。
- 2.1（完整模块所有权边界）和 2.6（runtime/planner/frontend/list surface 的全面 registry migration）仍未完成，未改变用户正在修改的 runtime/turn-flow 文件。
- 随后已完成 2.1/2.6 的基础范围：`ToolSurface`/`TurnToolView` 现在被 provider、capability bridge、planner 与 Tauri `list_available_tools` 共同消费；runtime 仍保留旧 `builtin_tools()` 仅作为兼容执行输入。
- 未改动用户的 `crates/pony-agent-core/src/agent/runtime/mod.rs`、`turn_flow.rs` markdown/model-hop trace 改动。
- Ask、Run、任意 URL WebFetch 仍未启用新行为，符合 fail-closed 决策。

## 2026-07-19 验证与审核状态

- `rustfmt --edition 2021`、`git diff --check` 与 `npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict` 通过。
- `npm run cargo:test:exact -- --lib agent::tools::contract_view_tests::` 仍在依赖编译阶段因 `link.exe not found` 失败，未执行项目测试代码。
- phase-2 两路独立代码审核已发起，但 reviewer 在大型 Rust 文件读取期间未在本会话内返回结论；因此不记录为审核通过，恢复后必须重试。

## 下一步

1. 在包含 MSVC `link.exe` 的开发者环境中运行新增 Rust 精确测试。
2. 实现 fake dispatcher/policy/budget harness，先用 `InvocationOrigin` 和 `TurnToolView` 覆盖 guessed internal/deferred 拒绝。
3. 仅在 runtime 最小接线点消费 `ToolOutcome.control_outcome`，再实现 `PendingControlRequest` 持久化和 Ask 恢复。

## Resume Hint

本轮的 blocker 与 Resume Hint 已被 `2026-07-26-pa076-phase2-closeout.md` 取代：MSVC 环境阻塞已解除，阶段 2 已收口。续跑请读那份日志与任务卡。
