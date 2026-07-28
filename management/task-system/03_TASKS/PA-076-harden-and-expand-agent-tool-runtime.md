# PA-076 加固并扩展 Agent Tool Runtime

## Basic Info

- ID: PA-076
- Status: In Progress
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-07-18
- Updated At: 2026-07-19
- OpenSpec Change: `harden-and-expand-agent-tool-runtime`
- Spec Status: 三方审核已修订并通过 strict validate

## Background

对 Pony Agent、Codex 与 Claude Code 当前工具实现的源码对比确认：Pony 已有 17 个内部定义和 13 个模型可见工具，但工具定义元数据存在多个真相源，`Plan / Ask` 仍是占位映射，组合子调用绕过统一治理，权限事实与真实副作用不一致，进程、Web 和搜索实现也缺少生产级资源与安全边界。

## Goal

建立可扩展、可审计的分层工具运行内核，先消除现有 P0/P1 风险，再补齐首批依赖该内核的基础工具，使未来 LSP、子代理和 workflow 不需要各自发明执行与权限通道。

## Scope

- 统一 tool descriptor / registry / dispatcher
- composite 子调用逐步治理与权限聚合
- 真实 `Plan`、`Ask` 与动态 `ToolSearch` 提升
- 进程生命周期、Web 安全、Search/Glob 语义加固
- `view_image` 与完整 MCP resource 基础面
- 测试、迁移、文档和观测读面同步

## Non-Goals

- 本卡不直接实现 LSP、Goal/Todo、worktree、子代理或 workflow
- 不重命名现有产品级工具名
- 不绕过现有 capability registry、hooks、trace 或任务系统创建第二套执行通道

## Acceptance Criteria

1. 所有模型可见、内部、deferred 和 composite 子调用都由统一 registry/dispatcher 解析与执行。
2. `ToolDefinition` 本身携带身份、schema、kind、exposure、权限、并发、取消和结果预算事实，不再由工具名散落推导。
3. `Plan` 不执行任意子调用；`Ask` 可由宿主暂停/恢复并在无宿主能力时结构化降级。
4. composite 权限是子步骤并集，且每个子步骤都产生独立 mediation、hook、permission 和 telemetry 证据。
5. `Run` 支持受控启动、轮询、stdin、终止和进程树清理，输出有稳定预算。
6. `WebFetch` 拒绝受限网络目标和危险重定向，并限制响应体与内容类型。
7. Search/Glob 使用真实 regex/glob/ignore 语义，扫描预算被显式报告而非静默漏结果。
8. `view_image`、MCP resource list/template/read 和 deferred 工具 turn 级提升可用并有测试。
9. Rust 定向/全量测试、静态检查、OpenSpec validate 和适用前端测试通过；安全与代码审核无未解决 P0/P1。

## Review Plan

- @architect：分层、依赖方向、迁移和组合执行边界
- @security-reviewer：命令、文件、SSRF、审批和宿主中介
- @test-engineer：失败矩阵、并发/取消/预算和回归门禁
- @code-reviewer-a / @code-reviewer-b：每个实现阶段完成后独立审核
- @consultant：最终收口裁决

## Current Progress

- 已完成三方工具源码对比和问题分级
- 已创建 OpenSpec change
- 已完成架构、安全、测试三路独立审核与 consultant 裁决；P0/P1 artifacts 已修订，strict validate 已通过，并补入 migration-aware characterization tests
- 已开始第 2 阶段：新增 `ToolDescriptor / ToolRegistrySnapshot`、`ToolOutcome` 兼容合同、`PendingControlRequest / TurnToolView / SandboxBackend / McpTransport` 独立模块；provider 已移除按工具数组长度推断 builtin 的逻辑，builtin capability 改读 descriptor 权限投影
- 第 2 阶段继续完成：权限声明已类型化；registry 拒绝保留命名空间、来源伪装、歧义 alias 与 composite cycle，并支持原子 source replacement；默认 provider、capability、planner、Tauri list-available surface 改由 `ToolSurface`/`TurnToolView` 投影；补入 `ProcessBackend`、resolver/MCP/clock fake harness。尚未接入旧 runtime。
- 第 2 阶段已收口（task 2.8 完成）：MSVC 环境阻塞已解除，新增 `scripts/run-rust-msvc.bat` 预加载 `vcvars64.bat`。首次真实执行 Rust 测试暴露并修复 3 个 phase-2 缺陷：
  1. registry 投影破坏了 provider 工具数组顺序（旧行为按 product name 首次出现的槽位排序，`time_now` 把 `Run` 钉在首位）。该顺序属于 PA-025/PA-029 收窄的 cache-friendly stable prefix，已改为在 builtin 构建时按「模型可见槽位」重排 descriptor，使 registry 位置成为所有投影的唯一顺序真相源。
  2. `model_visible_tool_name` 的 `_ => "Run"` 兜底会把任意未知/外部工具名静默改写成 builtin 产品名 —— 这是 tool-count 推断之外的第二条名称推断路径。已新增 `model_visible_tool_name_opt` 与 `product_visible_tool_name`，三处 contract view 改为未知名原样透传。
  3. 新测试对 `workspace_path_info` 断言产品名为 `Read`，与既有映射 `List` 冲突。phase-2 不得改变产品可见行为，故修正测试断言而非映射。

## Next Action

进入 task 3.1：实现 governed dispatcher lifecycle（origin 鉴权、raw validation、mutable hooks、final validation/normalization、final-argument policy 与 sandbox 决策、budgets、执行、control outcome、telemetry、exactly-once lifecycle hooks）。先补 fake dispatcher/policy/budget harness，再只在 runtime 最小接线点消费 `ToolOutcome.control_outcome`。禁止提前启用 Ask、Run 或任意 URL WebFetch。

## Blockers

- ~~缺少 MSVC `link.exe`~~ 已解除：Build Tools 实际位于 `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`，只是环境变量未加载。统一走 `scripts/run-rust-msvc.bat <命令>`。注意 `invoke-rust-target.ps1` 固定 `--manifest-path src-tauri/Cargo.toml`，跑 core crate 测试需显式 `cargo test -p pony-agent-core --lib --target-dir target-test-exact-a <filter>`。
- `runtime/mod.rs` 与 `turn_flow.rs` 有用户进行中的 markdown/model-hop trace 修改；PA-076 后续 dispatcher 接线必须在最小交叉点协同，当前未改动这两处。
- core 全量 `--lib` 另有 7 项失败与 PA-076 无关，不应计入本卡门禁：6 项 history/branch cursor 断言（`session` / `sqlite_session` / `control_plane` / `runtime` 的 node id 与 branch head 解析）与 1 项 `openai_reasoning_tool_followup_stream_attempts_live_stream_before_fallback`（不可达端点降级到 `local_fallback` 而非 `stream_sync_fallback`）。这些断言均不经过工具投影路径，疑与上述未提交的 runtime/turn_flow 改动或网络环境相关，建议单独定位。

## Validation Evidence

- `npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict`: passed.
- `git diff --check`: passed for the PA-076 worktree changes.
- `npm run cargo:test:exact -- --lib agent::tools::contract_view_tests::`: attempted through the required target-slot script; blocked while compiling dependencies because MSVC `link.exe` is unavailable. This is an environment blocker, not a passing test result.
- `rustfmt --edition 2021 <PA-076 Rust files>`、`git diff --check`、`npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict`: passed on 2026-07-19.

### 2026-07-26 (task 2.8 收口，首次真实 Rust 执行)

全部命令经 `scripts/run-rust-msvc.bat` 加载 MSVC 环境后执行：

- `cargo test -p pony-agent-core --lib --target-dir target-test-exact-a agent::tools::contract_view_tests::`
  首次执行：`9 passed; 3 failed` —— 暴露顺序回归与 path_info 命名冲突。
  修复后：`12 passed; 0 failed`。
- `cargo test -p pony-agent-core --lib --target-dir target-test-exact-a agent::tools::`: `65 passed; 0 failed`。
- `cargo test -p pony-agent-core --lib --target-dir target-test-exact-a agent::tool_runtime::`: `4 passed; 0 failed`。
- `cargo test -p pony-agent-core --lib --target-dir target-test-exact-a agent::provider::tests::`: `provider_does_not_infer_builtin_surface_from_tool_count` 首次失败（未知名被改写为 `Run`），修复后通过；同批 `33 passed`，另 1 项 followup fallback 失败属既有环境问题（见 Blockers）。
- `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression --target-dir target-test-exact-a`: `13 passed; 0 failed`。
- `rustfmt --edition 2021 crates/pony-agent-core/src/agent/tools.rs crates/pony-agent-core/src/agent/tool_runtime.rs`: passed。
- `git diff --check`: clean。
- `npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict`: `Change 'harden-and-expand-agent-tool-runtime' is valid`。

## Resume Hint

阶段 1、2 已全部完成。下次直接进入 task 3.1 的 governed dispatcher。

Rust 测试统一入口：`cmd //c "scripts\run-rust-msvc.bat cargo test -p pony-agent-core --lib --target-dir target-test-exact-a <filter>"`。

复跑本轮门禁：`agent::tools::`、`agent::tool_runtime::`、`--test tool_router_regression`。

接线 dispatcher 时注意两条顺序/命名不变量（本轮修复，回归代价高）：registry descriptor 顺序即 provider 工具数组顺序（stable prefix 依赖）；外部/未知工具名必须原样透传，不得回落到 builtin 产品名。
