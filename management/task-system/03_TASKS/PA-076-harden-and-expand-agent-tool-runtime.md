# PA-076 加固并扩展 Agent Tool Runtime

## Basic Info

- ID: PA-076
- Status: In Progress
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-07-18
- Updated At: 2026-08-02
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
- **第 3 阶段已全部完成（task 3.1–3.8，2026-08-02 收口）**：`GovernedDispatcher` 八步管线（origin 鉴权/exposure → schema 校验 → 可改写 hook → 最终校验+权限决策 → 原子预算 → 执行 → control outcome → exactly-once lifecycle record）、`PendingControlRequest` CAS（session/version/nonce/expiry/descriptor/digest 全绑定）、bounded child dispatch（lineage/深度 4/环检测/原子预算账本/`suspended` 停 sibling）、`workspace_batch` 只读门禁（3.4）、governed `workspace_batch`/`gather_context` composite + `GovernedToolExecutor` 适配器（3.5）、`aggregate_composite_permission` 保守聚合（3.6）、27 项矩阵测试（3.7）、独立安全+代码审核（3.8，CONDITIONAL PASS → P0/P1 全部解决）。
  - 测试：core lib 554 + matrix 27 + tool_router_regression 13 + session_regression 5 全绿；全工作区 check 通过。
  - 审核产物：`management/task-system/02_REVIEWS/2026-08-02-pa076-phase3-review.md`。
- **第 4 阶段已完成（task 4.1–4.5，2026-08-02 收口）**：`plan_state.rs` 的 `PlanStore`/`PlanControlHandler`（session-owned、revisioned，create/replace/merge/complete-step，`Draft/Executing/Completed/Aborted` 生命周期，绝不执行任意子调用）；`ask_control.rs` 宿主 Ask 适配器（复用 `PendingControlRequest` + `PendingControlRequestKind::Interaction`，answer 绝不等同 approval）；graph Ask wait/resume（`GraphAskWaitBinding` 持久化 per-run `ask_waits`、`bind_ask_wait`/`resume_ask_wait`、`waiting_user` 挂起、以原 tool call id 注入唯一终态）；control-plane `ask_plan_commands.rs` + 13 个 Tauri command（ask_list_pending/ask_answer/ask_cancel/ask_expire、plan_create/plan_replace/plan_merge/plan_complete_step/plan_list/plan_get、graph_list_ask_waits/graph_bind_ask_wait/graph_resume_ask）；前端 `src/stores/ask.ts`、`src/stores/plan.ts`、`AskPanel.vue`、`PlanPanel.vue`、`src/types/ask-plan.ts`。
- **第 5 阶段已完成（task 5.1, 5.3–5.6 核心落地）**：`process.rs` 的 `ProcessManager`（session-scoped opaque handle，start/poll/write_stdin/kill/kill_after/shutdown，并发排空 + bounded buffer + 截断/丢弃字节证据）；`sandbox.rs` 的 `SandboxSupportMatrix`/`NoSandboxBackend`/`TestSandboxBackend`（fail-closed 门禁）；`workspace_run_command` 已改由 `ProcessManager` 支撑；无人值守 Run 在无真实 sandbox backend 时按设计 fail-closed（`sandbox_unavailable`）。task 5.2 的 containment spike 决定已记录于 `sandbox.rs`（Windows Job Object 禁 breakaway 为意图 containment、Unix process group 为 best-effort，均未宣称完整 containment、Job Object 尚未实现为完整 enforcement）；task 5.7 审核仍属剩余工作。
- **第 6 阶段已完成（task 6.1–6.5 核心落地）**：`web_access.rs` 的 `WebAccessPolicy`/`PinnedConnector`/`WebAccessDecision`/`WebAccessDenyReason`（纯 hermetic 决策面、无网络 IO、无 HTTP client）；`web_fetch_url` fail-closed（`web_access_denied`、无 ambient proxy、不自动跟随 redirect）；`search.rs` 的 `SearchEngine`（真实 regex/globset/ignore 语义、确定性排序），`workspace_search_text`/`workspace_glob_files` 改用它并诚实报告截断（`truncated=true` + reason）。task 6.6 审核仍属剩余工作。
- **第 7 阶段已完成（task 7.1–7.4 核心落地）**：`image_artifact.rs` 的 `view_image`（reference-based `ImageArtifact`、magic-byte MIME/尺寸/字节上限、不解码像素）；`mcp_resources.rs` 的 MCP resource list / list-templates / read（source-bound `McpTransport`、untrusted content 边界、独立 `ResourceTemplate` 类型）；`tool_search_elevation.rs` 的 `ToolSearchElevator`（Deferred 候选 → 当前 turn `TurnToolView` 提升、source_revision 校验、trace evidence）。task 7.5 审核仍属剩余工作。
- **runtime 切换（task ②）已完成**：`AgentRuntimeBuilder` 默认 tool executor 现在是 `build_governed_executor`（`runtime/mod.rs` `build()` 的 `unwrap_or_else` 兜底；显式 `tool_executor(...)` override 仍优先）。保真测试证明 List/Search/Glob/Ask/Write/Edit 与 legacy 输出逐字节一致；Run 正确 fail-closed（`sandbox_unavailable`）。`tool_router_regression` 仍走 legacy `ToolRouter` 路径（13 项 characterization 门禁）。`DispatchError.code` 已动态化，任意 legacy 错误码可保留。
- **测试（最近一次验证快照）**：core lib 674 + matrix 27 + tool_router_regression 13 + session_regression 5 全绿；前端 vitest 328 全绿。
- **阶段 8 部分完成（task 8.1/8.2/8.4/8.5）**：8.1 已删死代码（`ToolCallContractView`/`ToolResultContractView`/`builtin_tool_contract_views` 投影 + 孤儿 helper + 死 `ToolRouter` 面）；8.2/8.5 已完成文档/canonical spec 同步（OpenSpec strict validate 通过）+ 任务系统同步；8.4 独立双审产出 `02_REVIEWS/2026-08-02-pa076-phases-4-7-review.md`（CONDITIONAL PASS，P1-2/3/4 + P2-10 已修复：预算回归、进程最小环境、WebFetch body 边界、time_now 类型；P1-1 Ask 真实接线与 P1-5 pinned connector 记录为下一里程碑）。

## Next Action

阶段 1–7 核心、runtime 切换（task ②）、阶段 8 的 8.1/8.2/8.4/8.5 已落地；core lib 675 + matrix 27 + 回归 + 前端全绿。剩余工作按依赖顺序：

1. **Ask 真实接线（P1-1，含 P2-8/P2-9）**：
   - ✅ 已落地（执行器级）：`persist_pending_request` 透传 Ask 问题到 `prompt`、`options` 数组到 `options`（P2-9）；`GovernedToolExecutor` 增加可设 session-scoped `DispatchContext`（`set_context`，P2-8）；governed 默认 evaluator 对 Ask 描述符返回 `WaitingHost` → Ask 现在真正持久化 `Interaction` `PendingControlRequest`（session 绑定 + 问题原样，测试 `governed_executor_ask_is_host_mediated_and_persists_the_question`）。
   - ⬜ 剩余（runtime turn-loop 接线）：运行时每轮设置 context（需把 governed executor 暴露给 runtime/control plane，单一共享 dispatcher）；检测 `control_outcome_pending` → 挂起 run + 调 `graph.bind_ask_wait`（graph 面已就绪）；host answer → `graph.resume_ask_wait` 注入唯一终态。
2. **pinned connector（P1-5/P2-7）**：移除"预校验后交默认 HTTP client 重解析"弱模式；WebFetch 连接固定到已校验地址 + peer-IP 校验 + 保留 Host/SNI + 显式 ≤5 跳重解析/重校验。`FailClosedResolver` 已兜住任意 hostname URL fail-closed，真实 resolver 接线前必须完成。
3. **真实 `SandboxBackend`**：`NoSandboxBackend` + `SandboxSupportMatrix` 是 fail-closed 门禁；注册真实 sandbox backend 后无人值守 Run 才从 `sandbox_unavailable` 转为执行。
4. **阶段 7 工具注册（P2-6）**：`view_image`/MCP resource/ToolSearch 提升的库已就绪，需注册进 builtin registry（`builtin:plan_control` 等）并处理 `ImageReadOptions.include_bytes` 默认 2 MiB 撑爆结果预算的隐患。
5. **阶段审核补完 + 8.3**：task 4.6/5.7/6.6/7.5 独立审核；8.3 格式化/静态检查/全量测试按门禁复跑，随后 OpenSpec 归档。

## Blockers

- ~~缺少 MSVC `link.exe`~~ 已解除：Build Tools 实际位于 `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`，只是环境变量未加载。统一走 `scripts/run-rust-msvc.bat <命令>`。注意 `invoke-rust-target.ps1` 固定 `--manifest-path src-tauri/Cargo.toml`，跑 core crate 测试需显式 `cargo test -p pony-agent-core --lib --target-dir target-test-exact-a <filter>`。
- `runtime/mod.rs` 与 `turn_flow.rs` 有用户进行中的 markdown/model-hop trace 修改。runtime 切换仅在最小交叉点落地（`AgentRuntimeBuilder::build()` 的默认 executor 兜底），未改动 `turn_flow.rs` 与用户进行中的 trace 修改。
- core 全量 `--lib` 另有 2 项既有失败与 PA-076 无关，不应计入本卡门禁：
  1. `provider_registry_regression::resolve_selection_falls_back_to_selected_provider_and_model`（config.rs 过期断言 4096 vs 64000）。
  2. `start_turn_stream_fail_turn_policy_on_tool_call_start_stops_before_tool_execution` 并行 flake（单线程通过）。

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

### 2026-08-02（阶段 1–7 + runtime 切换收口快照）

最近一次完整验证快照（全部通过；随后阶段 8.3 需按门禁全量复跑）：

- core lib `--lib`：**674 passed**（含 dispatcher/budget/dispatcher_composites/governed_executor/plan_state/ask_control/process/sandbox/web_access/search/image_artifact/mcp_resources/tool_search_elevation 等模块测试）。
- 矩阵 `--test dispatcher_matrix`：**27 passed**。
- `--test tool_router_regression`：**13 passed**（legacy ToolRouter characterization 门禁）。
- `--test session_regression`：**5 passed**。
- 前端 vitest：**328 passed**（含 `tests/ask-store.spec.ts`、`tests/plan-store.spec.ts`、`tests/AskPlanPanel.spec.ts`）。
- `npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict`：`Change 'harden-and-expand-agent-tool-runtime' is valid`（2026-08-02，canonical spec 同步后复跑，见下）。

## Resume Hint

阶段 1–7 与 runtime 切换（task ②）已全部完成并收口。架构说明见 `docs/architecture/tool-runtime-descriptor-registry.md`（含 governed dispatcher 八步管线、runtime 默认切换、Ask/Plan/process/sandbox/web/search/phase-7 模块落点与两条剩余 integrator notes）。阶段 3 审核产物：`02_REVIEWS/2026-08-02-pa076-phase3-review.md`。剩余工作：task 4.6 / 5.7 / 6.6 / 7.5 阶段审核、8.1–8.4 迁移与收口，以及两条 integrator notes（Ask 的 session-context 线程接线、真实 `SandboxBackend`）。

Rust 测试统一入口：`cmd //c "scripts\run-rust-msvc.bat cargo test -p pony-agent-core --lib --target-dir target-test-exact-a <filter>"`。

复跑门禁：`agent::dispatcher::`、`agent::budget::`、`agent::dispatcher_composites::`、`agent::governed_executor::`、`agent::plan_state::`、`agent::ask_control::`、`agent::process::`、`agent::sandbox::`、`agent::web_access::`、`agent::search::`、`agent::image_artifact::`、`agent::mcp_resources::`、`agent::tool_search_elevation::`、`--test dispatcher_matrix`、`--test tool_router_regression`、`--test session_regression`。

接线 dispatcher 时注意两条顺序/命名不变量（阶段 2 修复，回归代价高）：registry descriptor 顺序即 provider 工具数组顺序（stable prefix 依赖）；外部/未知工具名必须原样透传，不得回落到 builtin 产品名。

## OpenSpec 同步与归档就绪（8.5）

### Canonical spec 同步（已完成）

8 个 delta spec 已全量同步到 `openspec/specs/`：

- 新增 canonical：`tool-runtime-dispatch`、`process-tool-lifecycle`、`web-access-safety`。
- 扩展 canonical（追加本 change 的 ADDED 要求）：`tool-system-contract`（+4）、`tool-permission-contract`（+5）、`third-wave-default-tool-alignment`（Plan/Ask 两条 MODIFIED 已替换 + ToolSearch elevation ADDED）、`second-wave-tool-surface`（+5）、`tool-observability-contract`（+4）。

`openspec/changes/harden-and-expand-agent-tool-runtime/tasks.md` 勾选状态已与实际实现对齐。

### 验证

- `npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict`：`Change 'harden-and-expand-agent-tool-runtime' is valid`。

### 归档就绪状态（未执行归档）

change 仍留在 `openspec/changes/harden-and-expand-agent-tool-runtime/`，**尚未归档**。归档前的剩余前置（完成后再执行 `openspec/changes/archive/2026-08-02-*` 迁移并更新 `docs/INDEX.md` 链接）：

1. task 4.6 / 5.7 / 6.6 / 7.5 独立阶段审核（阶段 3 审核已完成：`02_REVIEWS/2026-08-02-pa076-phase3-review.md`；其余阶段尚无审核产物）。
2. task 8.1：删除已迁移的旧名称派生表/执行旁路（legacy `builtin_tools()`/`ToolRouter` 仍作执行/兼容输入与 characterization 门禁）。
3. task 8.3：格式化、静态检查、core 定向/全量 + 前端测试按门禁全量复跑（最近一次快照：core lib 672 + matrix 27 + tool_router_regression 13 + session_regression 5 + 前端 vitest 328）。
4. task 8.4：独立双路代码审核 + 安全/性能审核 + consultant 收口，无未解决 P0/P1。
5. 两条 integrator notes 若在归档前完成接线（Ask session-context、真实 `SandboxBackend`），则删除本卡相应"剩余"标注。

归档时的预期动作：将 `openspec/changes/harden-and-expand-agent-tool-runtime/` 迁入 `openspec/changes/archive/2026-08-02-harden-and-expand-agent-tool-runtime/`，更新 `docs/INDEX.md` 第 8 节链接（canonical spec 链接保持指向 `openspec/specs/`，不受归档影响），并在 Dashboard/Board 将该卡标记为 Done。
