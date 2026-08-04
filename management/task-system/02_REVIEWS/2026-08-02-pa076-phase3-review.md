# PA-076 Phase 3 Review — Governed Dispatcher and Permissions

## 审核对象

- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/design.md)（Decisions 1–5、Verification Strategy、Migration Plan）
- [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/tasks.md) 任务 3.1–3.7
- 阶段 3 产物：
  - `crates/pony-agent-core/src/agent/dispatcher.rs`（`GovernedDispatcher` 八步管线、授权、CAS、child runner、权限聚合）
  - `crates/pony-agent-core/src/agent/budget.rs`（原子预算账本、CancellationToken）
  - `crates/pony-agent-core/src/agent/child_dispatch.rs`（bounded child dispatch、`dispatch`/`dispatch_many`）
  - `crates/pony-agent-core/src/agent/dispatcher_composites.rs`（governed `workspace_batch`/`gather_context`、`GovernedToolExecutor`）
  - `crates/pony-agent-core/src/agent/tools.rs`（task 3.4 只读门禁、阶段 2 契约）
  - `crates/pony-agent-core/tests/dispatcher_matrix.rs`（矩阵测试 27 项）

## 审核阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@security-reviewer` | **CONDITIONAL PASS** | 沙箱门在无 backend 时 fail-open；默认 evaluator 对 Write/Execute 全 Allow；pending 后停 sibling 仅在 `dispatch_many` 生效；digest 绑定但消费不校验；`dispatch_many` 线程数无上限 | P1-1 / P1-2 / P1-3 本轮修复；P2 digest 校验、请求数上限本轮修复 |
| `@architect-code` | **CONDITIONAL PASS** | composite 输出契约三处偏差（外层 status 恒 ok、错误码塌缩为 `handler_error`、`permissionSummary` 硬编码 WorkspaceRead）；lifecycle observer 持锁迭代可毒化 dispatcher；`concurrent_safe` 未执行 | P2-1 / P2-2 / P2-3 / P2-4 本轮修复；P2-5 推迟到 runtime 切换里程碑 |

无 P0。两份审核均确认：CAS 原子性（单 Mutex 全程持锁）、hook 无法改写 identity/origin（签名约束）、composite 无法自授子权限（`ChildDispatch::new` 为 pub(crate)）、`reserve_bytes` 原子回滚、panic 时 `ConcurrencyGuard` 释放，均健壮。

## 本轮已修复（代码变更）

### P1-1 沙箱门 fail-open（security）
- 修复：`requires_sandbox(descriptor)` 为真且无已注册 sandbox backend（或 `Unavailable`）时返回 `sandbox_unavailable`，不再静默跳过；沙箱门仅作用于 `Allow` 判定，避免掩盖 `permission_denied`/审批流。
- 测试：`execute_descriptor_without_sandbox_backend_fails_closed`。

### P1-2 默认 evaluator 对 Write/Execute 全 Allow（security）
- 修复：`DescriptorPolicyEvaluator` 保守化——声明 `WorkspaceWrite`/`WorkspaceExecute` scope 的描述符默认提升为 `ApprovalRequired`（除非宿主注册更精确的 evaluator），并记录 `reason`。Read/discovery 仍 Allow。
- 测试：新增/既有审批测试覆盖。

### P1-3 pending 后停 sibling 仅在 `dispatch_many` 生效（security）
- 修复：`ChildDispatchContext` 增加共享 `suspended` 标志；单次 `dispatch()` 与串行路径在任一 child 进入 pending 控制状态后，后续 sibling 一律返回 `not_started`（aborted）。
- 测试：`single_child_dispatch_stops_siblings_after_a_pending_control_child`。

### P2 控制请求参数摘要不校验（security / design.md Decision 5）
- 修复：`ControlRequestAuthorization` 增加可选 `expected_final_args_digest` / `expected_policy_digest`，`consume_control_request` 校验，不匹配 fail closed。
- 测试：`control_request_rejects_final_args_digest_mismatch`。

### P2-1 composite 外层 `ToolResult.status` 恒 ok（architect）
- 修复：`GovernedToolExecutor::execute` 在 payload `ok == false` 时把外层 `status` 置为 `error`，消费方按 `status != "ok"` 判定不再误读为成功。

### P2-2 batch/gather 错误码塌缩为 `handler_error`（architect）
- 修复：新增 `handler_failure`，`code: message` 前缀中的已知码（`missing_argument`/`too_many_calls`/`invalid_call_shape`/`out_of_scope`/`invalid_path`/`aggregate_failed`）保留为结构化 `error.code`；`run_handler` 与 child composite 路径均使用。
- 测试：更新 composites 断言为 `error.code`。

### P2-3 `permissionSummary.scopes` 硬编码 WorkspaceRead（architect / security）
- 修复：`ChildDispatch` 新增只读 `registry()` 访问器；`aggregate_nested_entries` 从子描述符声明 scope 派生并集（并保留 composite 自身 read scope）。
- 注：真实强制仍按 child 逐个授权，此为证据完整性修复。

### P2-4 lifecycle observer 持锁迭代可毒化 dispatcher（architect）
- 修复：`emit` 先克隆 observer 列表再迭代，panicking/re-entrant observer 不再毒化共享锁。

### P2 `dispatch_many` 线程数无上限（security）
- 修复：新增 `MAX_DISPATCH_MANY_REQUESTS = 256` 上限，超限返回错误，限制并发建线程数。

### P3 杂项
- `budget.rs`：`max_concurrency == 0` 时 `ConcurrencyGuard` 不再下溢计数器（`active` 标志）。
- `child_dispatch.rs`：panic 的 child 保留真实 request 身份（不再回退 `"unknown"`）。
- `dispatcher.rs`：`child_self_recursion` 分支注明为防御性冗余（lineage 先命中 `child_cycle`）。
- `dispatcher_matrix.rs`：清理 Part 2 陈旧的 `#[ignore]` 头部注释。

## 推迟项（记录在案，按里程碑承接）

| 发现 | 处置理由 | 承接点 |
| --- | --- | --- |
| 内置权限声明全部 `requires_approval=false` + runtime 默认 exec path 的审批语义 | 与"runtime 切换"强绑定：切换时需注册真实 policy evaluator + 校准声明；默认 evaluator 已保守化兜底 | runtime 切换里程碑 |
| `concurrent_safe` 未执行（P2-5） | 当前无真实 handler 注册，flag 为死元数据；切换时按串行/并发分组落地 | runtime 切换里程碑 |
| `host_control_available` 默认 true（P3） | 改动默认会破坏"dispatch() 返回 pending"契约；headless 由 runtime 显式设置 | runtime 切换里程碑 |
| `GovernedToolExecutor` 丢弃 session 事实（P3-4） | 适配器定位为 harness seam；切换时用 `dispatch_governed` + 真实 context | runtime 切换里程碑 |
| `WorkspaceWrite/Execute` 提升覆盖 `permission_denied` 的沙箱顺序、WaitingHost 以 Approval kind 持久化（P3） | 需 Ask/approval 语义定型 | 阶段 4（Plan/Ask） |
| schema 校验忽略 pattern/enum 等关键字（P2） | 需先审计内置 schema 是否使用这些关键字，避免误拒 | 阶段 7（MCP/skill descriptor 一等化） |
| 非ce 顺序可预测、`pending_requests` 无界增长（P3） | 进程内信任边界；session 层负责 prune/持久化 | session 持久化收口 |
| composite 输出字节预算双重计数（P3-7） | 保守（fail-closed），接受 | 文档化 |
| 剩余测试缺口（deadline 走真实时钟、并发预算争用、panic handler）（P3-3） | 低风险补强 | 后续补测 |
| record 顺序 child-before-parent、顶层用 raw 名（P3-5/6）、路径等值（P3-9）、双层 limit（P3-10） | 语义已文档化，切换时补 characterization | runtime 切换里程碑 |

## 测试证据

- `cargo test -p pony-agent-core --lib --test dispatcher_matrix -- --test-threads=1`：**549 + 27 全通过，0 失败**
- `tool_router_regression`：13/13 通过（含 task 3.4 门禁）
- `session_regression`：5/5 通过
- `npm run cargo:check:shared`（全工作区）：通过
- 既有非本轮问题（已记录）：`provider_registry_regression::resolve_selection_*`（config.rs 过期断言）；runtime 并行 flake（单线程通过）

## 结论

阶段 3（3.1–3.7）可判定为 **通过**；P0/P1 已全部解决（含一处文档化采纳）。`GovernedDispatcher` 已具备成为 runtime 工具执行引擎所需的 fail-closed 治理边界；剩余 P2/P3 已记录并绑定到 runtime 切换里程碑 / 阶段 4 / 阶段 7。
