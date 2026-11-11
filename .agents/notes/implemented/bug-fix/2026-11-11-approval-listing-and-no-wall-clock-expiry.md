# Agent Note: 修复非路径审批列表过滤与执行侧去墙钟闭环 (ISSUE-20261111-001 & ISSUE-20261111-002)

Status: implemented

## Context

在真实交互中上报了两项影响审批可用性的 P1 缺陷：
1. **ISSUE-20261111-001**：非路径类审批（如规则拦截、Draft plan 的 `start_execution` 执行审批，对应 `approvalKind="approval"`）在触发时，虽然弹出了审批面板，但宿主控制面 `approval_is_listable` 硬编码了必须是路径授权类审批（`is_path_authorization_approval`）。导致 500ms 内宿主未决列表同步对账时找不到该项，前端/TUI 判定请求已 stale 并强制关闭面板，误报 `APPROVAL_STALE_NOTICE`（“该审批已过期，本轮不会自动继续；请重新发起该操作”）。同时 `turn_control.rs` 的 `suspension_is_unrecoverable` 同样排除了非路径审批。
2. **ISSUE-20261111-002**：架构决策 ADR 0074 已经确定全面去墙钟化（待决请求由显式决策、新消息顶替或会话 GC 回收，`expires_at_ms` 仅作为追踪元数据）。但在执行侧 `governed_executor.rs` 中，`plan_execution_approval` 仍保留了 `now_ms < approval.expires_at_ms` 判据（默认配置仅 60s），导致人工慢决策（停留超过 1 分钟）点击同意后被系统误拒并报错过期。

## Decision

彻底打通审批的“可列出”与“合法消费”闭环，对齐 ADR 0074 去墙钟原则：

1. **放行非路径审批的可列出性（解 001）**：
   - `crates/pony-agent-core/src/agent/control_plane/mod.rs`：修改 `approval_is_listable` 判据，只要请求种类为 `Approval` 且状态为 `Pending` 即可列出，消除对非路径成因的过度过滤。
   - `crates/pony-agent-core/src/agent/runtime/turn_control.rs`：修改 `suspension_is_unrecoverable`，只要宿主开启审批恢复且审批携带会话身份，即为可恢复挂起。
2. **执行侧全面去墙钟化（解 002）**：
   - `crates/pony-agent-core/src/agent/governed_executor.rs`：移除 `now_ms < approval.expires_at_ms` 校验，计划执行审批凭证只要会话与计划匹配即合法放行。
   - `crates/pony-agent-core/tests/wave36_plan_execution_approval.rs`：同步将历史断言超时的旧测试用例升级为去墙钟验收用例 `c141_5_approval_past_wall_clock_expiry_still_unlocks`。

## Alternatives considered

- **为非路径审批增加专有 approvalKind 白名单**：
  落选。未来可能扩展各种类型的审批（包括插件审批、命令风险审批、工具权限抬升），白名单只会持续制造遗漏和谎报；只要是合法的待决 `Approval`，都应当允许在宿主面板上列出并供用户决议。
- **调大 `control_request_expiry_ms`（例如从 60s 增加到 1 小时）**：
  落选。任何固定的物理墙钟阈值都会误伤慢人工决策，且与 ADR 0074“全面去墙钟”架构原则相悖。审批生命周期属于会话状态，应当且仅当用户显式做出决议（允许/拒绝）、新消息覆盖或会话归档时终结。

## Verification

- **契约矩阵**: `.dev-team/contract-matrix-approval-fixes.md`（C-APP-01 ~ C-APP-05 全部通过）
- **测试用例**:
  - `agent::control_plane::wave65_r2_approval_tests` (3 passed; 0 failed)
  - `agent::runtime::turn_control::w1_approval_unrecoverable_tests` (2 passed; 0 failed)
  - `agent::governed_executor::tests::governed_executor_plan_execution_approval_valid_past_wall_clock_expiry` (1 passed; 0 failed)
  - `tests/wave36_plan_execution_approval.rs` (9 passed; 0 failed)
  - `tests/approval_resume_e2e.rs` (4 passed; 0 failed)
  - `tests/tui_batch3_regression.rs` (137 passed; 0 failed)
