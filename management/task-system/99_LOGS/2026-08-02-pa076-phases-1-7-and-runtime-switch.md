# 2026-08-02 PA-076 阶段 1–7 与 runtime 切换收口

## 本轮目标

- 将 `PA-076 / harden-and-expand-agent-tool-runtime` 的阶段 1–7 与 runtime 切换（task ②）状态同步到文档体系：
  - `docs/architecture/tool-runtime-descriptor-registry.md`（governed dispatcher 八步管线、runtime 默认切换、新模块落点、两条 integrator notes）
  - `docs/INDEX.md`
  - 任务卡 / Dashboard / Task Board
  - `openspec/specs/` canonical specs（8 个 delta spec 全量同步）
  - 归档就绪备注（不实际归档）
- 全程仅改 `.md`，不触碰 Rust/TS 源码。

## 阶段完成事实（与代码核对）

- **阶段 3**：`GovernedDispatcher` 八步管线、`PendingControlRequest` CAS、bounded child dispatch、governed composites、27 项矩阵测试；独立审核 CONDITIONAL PASS → P0/P1 全解（`02_REVIEWS/2026-08-02-pa076-phase3-review.md`）。
- **阶段 4**：`plan_state.rs`（`PlanStore`/`PlanControlHandler`）、`ask_control.rs`（宿主 Ask 适配）、graph `ask_waits` wait/resume、control-plane `ask_plan_commands.rs`、13 个 Tauri command、前端 `ask.ts`/`plan.ts`/`AskPanel.vue`/`PlanPanel.vue`/`ask-plan.ts`。
- **阶段 5**：`process.rs`（`ProcessManager`）、`sandbox.rs`（`SandboxSupportMatrix`/`NoSandboxBackend`/`TestSandboxBackend`）；`workspace_run_command` 由 `ProcessManager` 支撑；Run 无真实 sandbox 时 fail-closed（`sandbox_unavailable`）。
- **阶段 6**：`web_access.rs`（`WebAccessPolicy`/`PinnedConnector`/`WebAccessDecision`/`WebAccessDenyReason`）；`web_fetch_url` fail-closed（`web_access_denied`、无 ambient proxy、不自动跟随 redirect）；`search.rs`（`SearchEngine`，真实 regex/globset/ignore）。
- **阶段 7**：`image_artifact.rs`（`view_image`）、`mcp_resources.rs`（MCP resource list/templates/read via `McpTransport`）、`tool_search_elevation.rs`（`ToolSearchElevator`）。
- **runtime 切换（task ②）**：`AgentRuntimeBuilder` 默认 tool executor = `build_governed_executor`（`runtime/mod.rs` `build()` 兜底）；`tool_router_regression` 仍走 legacy `ToolRouter`。

## 最近一次验证快照

core lib 672 + matrix 27 + tool_router_regression 13 + session_regression 5 + 前端 vitest 328，全绿。

## OpenSpec canonical sync（8.5）

- 新增 canonical：`tool-runtime-dispatch`、`process-tool-lifecycle`、`web-access-safety`。
- 扩展 canonical（追加本 change 的 ADDED / 替换 MODIFIED）：`tool-system-contract`（+4）、`tool-permission-contract`（+5）、`third-wave-default-tool-alignment`（Plan/Ask 替换 + ToolSearch 追加）、`second-wave-tool-surface`（+5）、`tool-observability-contract`（+4）。
- `tasks.md` 勾选状态已与实际实现对齐（3.x–7.4、8.2、8.5 已完成；4.6/5.7/6.6/7.5 审核与 8.1/8.3/8.4 未完成）。
- **归档未执行**：见任务卡 "Archive Readiness (8.5)" 一节。

## 剩余工作（下轮）

1. task 4.6 / 5.7 / 6.6 / 7.5 独立阶段审核。
2. 两条 integrator notes：Ask 的 session-context 线程接线（`GovernedToolExecutor` 目前是 harness seam，需 runtime 用 live context 调 `dispatch_governed`）；真实 `SandboxBackend`（无人值守 Run 的 fail-closed 门禁待替换为真实实现）。
3. task 8.1（清理旧名称派生表/旁路）、8.3（格式化/静态检查/全量复跑）、8.4（双路代码审核 + 安全/性能审核 + consultant 收口），随后执行归档。
