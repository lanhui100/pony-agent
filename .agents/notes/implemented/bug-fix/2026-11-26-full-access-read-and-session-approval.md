# Agent Note: `full_access` 放行工作区外读取与 TUI「本会话允许」按会话隔离生效

Status: implemented

## Problem

1. **完全访问（`full_access`）下读工作区外文件仍弹出审核弹窗**：
   - `GovernedPolicyEvaluator::evaluate_with_context` 只对 `read_only` 模式拦截写/执行类工具，`full_access` 与 `workspace_write` 完全等价；进入步骤 3 路径安全门禁后仍触发 `ApprovalRequired`。
   - `PrimitiveToolHandlerRequest` 与 `ToolExecutionContext` 未携带 `tool_authorization_mode`，底层 `ToolRouter`、`ReadDocumentHandler`、`ViewImageHandler` 无法感知 `full_access`，执行层也会以 `requires_authorization` 拒绝工作区外读取。
2. **TUI 选择「本会话允许」（`AllowSession`）后仍重复弹窗**：
   - `approval_plan` 与 `apply_approval_decision` 把 `AllowOnce` 和 `AllowSession` 合并在同一分支，均只调用 `authorize_path(path, "read")`，仅放行当前单文件而不覆盖工作区外其他文件；同时持久化进全局 `path_authorizations.v1`。
   - 非路径类审批（`tool_rule_ask` / `external_requires_approval`）在 `AllowSession` 时没有任何会话级允许表记录，导致后续调用同一工具继续弹窗。

## Decision

1. **`AuthorizeStore` 新增仅内存、按 `session_id` 隔离的会话级允许表**（`crates/pony-protocol/src/path_auth.rs`）：
   - `session_external_read_all: RwLock<HashSet<String>>` 与 `session_allowed_descriptors: RwLock<HashSet<(String, String)>>`；
   - 不进入 `entries()`、`snapshot_map()` 与持久化回调；
   - 空 `session_id` / `descriptor_id` 一律 fail-closed 拒绝；提供 `grant_session_external_read` / `has_session_external_read` / `grant_session_tool_approval` / `has_session_tool_approval` / `revoke_session_grants`，经 `SessionStore::path_authorizations()` 既有访问器与 `HostControlPlane::authorize_session_*` 暴露（不在 `SessionStore` 上新增转发方法——store.rs 已触文件规模棘轮）。
2. **全链路透传 `tool_authorization_mode` 并放行工作区外读取（保持 2A 写保护边界）**：
   - `PathPermissionChecker::classify_with_options(..., allow_external_read)` 仅在 `PathPurpose::Read` 未命中工作区/tmp/显式授权/技能根且 `allow_external_read == true` 时返回 `PermissionZone::AuthorizedExternal`；`PathPurpose::Write` 边界保持不变（外部写仍返回 `OutsideWorkspaceWriteDenied`）。
   - `DispatchContext` → `PrimitiveToolHandlerRequest` → `ToolExecutionContext` 透传 `tool_authorization_mode`；两条派发路径（直接 handler / pinned surface runner）统一经 `PrimitiveToolHandlerRequest::from_dispatch_context()` 装配（会话 / workspace / turn 令牌 / 授权模式单点透传，避免新增字段只改一处），再由桥接层透传给 `ToolExecutionContext`；`GovernedPolicyEvaluator`、`ToolRouter`（`paths.rs`）、`ReadDocumentHandler`、`ViewImageHandler` 在 `tool_authorization_mode == "full_access"` 或 `has_session_external_read(session_id)` 时放行工作区外读取。
   - `GovernedPolicyEvaluator` 在命中 `RuleAction::Ask` 或外部工具 `requires_approval` 时，若命中 `has_session_tool_approval(session_id, descriptor_id)` 则放行（`decision_source = "session_tool_approval"`）；`RuleAction::Deny` 不可绕过。三处判定收敛到 `session_tool_approved()` / `tool_rule_ask_decision()` / `path_gate_decision()` 三个私有 helper，`decide_with_context` 因此重新落回文件规模棘轮的单函数行数上限之内（200 行）。
3. **TUI 区分 `AllowOnce` 与 `AllowSession` 语义**（`crates/pony-tui/src/tui.rs`、`protocol.rs`、`projection.rs`、`permission_panel.rs`）：
   - `AllowOnce`：路径审批仅写入单路径 `authorize_path(path, "read")`，无路径审批直接 CAS；
   - `AllowSession`：路径审批调用 `authorize_session_external_read(&session_id)`（本会话放行所有工作区外读取，不污染全局持久表）；非路径审批调用 `authorize_session_tool(&session_id, &descriptor_id)`（本会话放行该工具）；
   - 同步修正面板说明文案与 `headless.rs` / `crates/pony-cli/src/main.rs` 中关于 `full_access` 等价性的过时说明。

## Verification

- `bash scripts/cargo-lane.sh check --workspace --all-targets` → exit 0（2m44s / 重跑 21s）
- `bash scripts/cargo-lane.sh test -p pony-tools -p pony-storage -p pony-tui` → 324 / 505 / 298 / 486 passed，0 failed（含 doctest）
- `bash scripts/cargo-lane.sh test -p pony-agent-core --test session_storage_suite` → 187 passed，0 failed
- `bash scripts/cargo-lane.sh test -p pony-agent-core --test tools_stop_suite` → 404 passed / 1 failed；
  失败例 `wave37_tool_surface::t3_h_subagent_with_stub_engine_returns_real_output` **单独复跑通过**，
  与 `runtime_suite` 既有的 `SUBAGENT_ENGINE` 进程级 `OnceLock` 抢注同类（基线可复现，见
  `.agents/notes/implemented/bug-fix/2026-11-26-test-support-default-data-dir-isolation.md` 的残留风险条），
  已按环境缺陷登记待办
- `node scripts/check-file-size.mjs --baseline scripts/file-size-baseline.json` → 0 new violation
  （未上调基线，靠抽取 helper 与单点装配压回）
- `npm run adr:check` / `npm run backlog:gate` → PASS

## Alternatives considered

- **在 `AllowSession` 时将路径所在父目录（或根目录）写入持久化的 `authorize_path`**：会把单次会话意图永久持久化到跨会话全局表 `path_authorizations.v1`，违背「仅本会话允许」语义，且无法覆盖非路径类审批。
- **`full_access` 下同时放开工作区外写入**：会突破 ADR 0029 外部写须显式 `read-write` 授权的安全边界，超出本次问题范围，故保持 2A 仅放行读取。
