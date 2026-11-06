# wave-65 green 收据：审批类控制请求可恢复（approval-resume）

> 契约真值源：`.dev-team/contract-matrix-approval-resume.md`（frozen，C1~C4）
> 红相验收（只读，未改动）：`crates/pony-agent-core/tests/approval_resume_e2e.rs`
> 实施者：Executor（wave-65 T3）｜时间：2026-11-06

## 0. 修改文件

| 文件 | 契约 | 落点 |
| --- | --- | --- |
| `crates/pony-agent-core/src/bin/tui.rs` | C1 / C4 | `build_control_plane` 内 `AgentRuntimeBuilder::desktop().host_approval_resume(true)`；`sync_pending_control_request` 合并 `list_pending_approvals`；`approval_continuation_prompt` / `apply_approval_decision` 实现；`UiEvent::ApprovalResumeTurn` 变体 + 事件循环分支；PermissionApproval 按键分支改走纯接缝；`#[cfg(test)] mod wave65_approval_unit_tests`（2 例） |
| `crates/pony-agent-core/src/agent/dispatcher.rs` | C2 | `persist_pending_request` 的 `ApprovalRequired` prompt/options；新增 `approval_path_from_arguments` / `approval_options_from_arguments` / `approval_prompt_from_arguments`（Ask 路径 `ask_options_from_arguments` 逐字未动） |
| `crates/pony-agent-core/src/agent/control_plane/mod.rs` | C3 | `list_pending_approvals`（会话作用域 + Approval/Pending 过滤 + 无 nonce 投影）、`cancel_approval_request`（CAS → `Cancelled`）、`approval_pending_projection` |
| `crates/pony-agent-core/src/agent/control_plane/ask_plan_commands.rs` | C3 | `require_current_version` 提升为 `pub(super)`（供 `cancel_approval_request` 复用同一 CAS 版本闸门，语义逐字不变） |

未改动（纪律核对）：`tests/**`（含冻结的 `approval_resume_e2e.rs`、并行会话的 `tests/tui_terminal_restore.rs`）、`src-tauri/**`、`.dev-team/**`（本收据除外）、`bin/tui.rs` 内 thinking-bottom-gap 并行会话区域（`MessageRole::Thinking` / `StreamReasoning` / `apply_stream_reasoning` / `stream_delta_ui_payload` / `CHAT_BOTTOM_GAP`）。

## 1. 门禁原文（命令 + Exit Code）

### 门禁 1：`cargo check -p pony-agent-core --bins`
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.26s
GATE1_EXIT=0
```

### 门禁 2：`cargo test -p pony-agent-core --test approval_resume_e2e`
```
test t1_host_approval_resume_suspends_outside_workspace_read_and_lists_the_pending_approval ... ok
test t3_cancel_approval_request_is_terminal_and_approve_after_cancel_fails_closed ... ok
test t2_approval_then_authorized_continuation_turn_completes_the_read ... ok
test t4_default_runtime_fails_closed_with_approval_resume_unavailable ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.97s
GATE2_EXIT=0
```
逐题结论：T1 ✅（C1+C3：`turn:suspended` 恰 1 / `turn:failed` 恰 0；`list_pending_approvals` 恰 1 且 `options.path` == 越界原始路径、无 `nonce`）；T2 ✅（C3+C4：`authorize_path`+`approve_control_request` 后续轮 `turn:completed` 恰 1，续轮请求体不含 `control_outcome_pending`）；T3 ✅（INV-4：cancel 后同版本 approve 必 Err，拒绝后不在待决列表）；T4 ✅（INV-2：默认 `false` ⇒ `turn:failed` 恰 1 且 error 以 `approval_resume_unavailable` 开头、`turn:suspended` 恰 0）。

### 门禁 3：`cargo test -p pony-agent-core --lib turn_control`
```
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1308 filtered out; finished in 0.00s
GATE3_EXIT=0
```

### 门禁 4：`cargo test -p pony-agent-core --test ask_resume_e2e --test permission_fail_closed_contract`
```
test result: ok. 8 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.72s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.76s
GATE4_EXIT=0
```

### 门禁 5：`cargo test -p pony-agent-core --lib`（基线 1316 passed / 0 failed）
```
test result: ok. 1316 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 42.65s
GATE5_EXIT=0
```
结论：与基线逐字一致（1316 / 0），无新增失败、无新增 ignored。

### 附加（TUI 单测，C4 要求）
```
cargo test -p pony-agent-core --bin pony
test wave65_approval_unit_tests::approval_continuation_prompt_is_non_empty_and_distinct ... ok
test wave65_approval_unit_tests::apply_approval_decision_without_pending_handle_returns_none ... ok
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
BIN_EXIT=0
```

### 门禁 6：`cargo fmt -p pony-agent-core --check` → **基线即红，报告并跳过**
```
FMT_EXIT=1     # 6648 个 "Diff in"，跨 ~200 个文件（含大量本波未触碰的文件）
```
- 证据：`crates/pony-agent-core/src/bin/tui.rs` 在 **HEAD（基线）** 上单独跑 `rustfmt --check` 已有 141 处差异
  （本仓历史即未按 rustfmt 落盘，例如 `AskResumeTurn { run_id: String },` 这类单行结构体变体全仓一致）；
  本波新增代码刻意沿用该文件既有风格（`ApprovalResumeTurn { prompt: String },`）。
- 因此本门禁不作为本波判据；按任务书「若整仓 fmt 因并行会话失败，报告并跳过」处理。**未对任何文件执行 `cargo fmt` 改写**（避免触碰并行会话区域）。

## 2. 冲突 / 需上报 Lead 的事项

1. **无契约冲突**：C1~C4 均按冻结契约实现；红相测试 `approval_resume_e2e.rs` 全绿且**未改动一行**。
2. **键位语义的一处收紧（契约明确要求，非回归）**：旧 `y/Y` 分支只 `approve_control_request`（不写路径授权），
   旧 `Enter` 仅在 `AllowSession` 时 `authorize_path`。现按契约 C4 统一为「`AllowOnce|AllowSession` 且
   `command_or_path` 存在 ⇒ 先 `authorize_path(path,"read")` 再 approve」，仍**只写 read 作用域**（INV-1 不变）。
3. **旧审批拒绝路径的缺陷被一并修正**：原 `Esc/n` 与 `Enter+Deny` 在审批态调用的是 `cancel_ask_in_session`
   （Ask 持久化 store 的 CAS，对 Approval 恒失败），现改走 `cancel_approval_request`（dispatcher CAS → `Cancelled`），
   因此错误文案不再出现「取消审批失败: ask_not_found」。
4. **`bin/tui.rs` 既有未提交改动**：`todo!("契约 C6…")` 已被并行会话（thinking-bottom-gap）替换为真实实现，
   属其未提交工作，本波**未触碰**该区域；`tests/tui_terminal_restore.rs` 的 178 行改动亦非本波产物。
5. **可观测性收口**（契约 §3）未在本波执行：新增宿主读取接口 `list_pending_approvals` / `cancel_approval_request`
   的 `docs/observability.yaml` 四维归属与 `audit-diff.sh --base ebf50d64` 属审查波次门禁，交由 Lead 安排。

## 3. 关键设计落点（便于复核）

- **C1**：`tui.rs::build_control_plane` 唯一消费点打开能力位；`AgentRuntime` 默认值仍为 `false`（INV-2 零回归）。
- **C2**：`options` = `{approvalKind:"path_authorization",tool,path,scope:"read"}`（无 path ⇒ `{approvalKind:"approval",tool}`）；
  prompt = `工具 Read 需要读取工作区外路径 /x/y，请授权后重试。`；`path` 取**原始**串（不 trim、不归一化），
  保证「展示的路径」== 「`authorize_path` 授权的路径」== 「快照 `options.path`」三处一致。
- **C3**：仅读 `ask_dispatcher.pending_requests()` 内存 CAS store（审批无持久化 store），
  投影字段恰为 11 项、**不含** `nonce`/`finalArgsDigest`/`policyDigest`；空/空白 scope fail-closed + eprintln。
- **C4**：`apply_approval_decision` 是唯一决策面（按键分支、单测共用）；`UiEvent::ApprovalResumeTurn` 复用
  `AskResumeTurn` 的整套轮次线程基建，差异仅两点：文案来自事件、入口用 `start_ask_aware_turn_stream`
  （审批续轮无 run 绑定）；**不** `add_user_message`（无用户气泡）。

---

# R2 收据（四路对抗审查 FAIL 修复，Lead 裁决）

> 契约：`.dev-team/contract-matrix-approval-resume.md` §2 C1/C3/C4/C5 的 R2 增补（F1~F4）
> 实施者：Executor（wave-65 task-5）｜时间：2026-11-06

## R2-0. 修改文件

| 文件 | 修复项 | 落点 |
| --- | --- | --- |
| `crates/pony-agent-core/src/agent/runtime/turn_control.rs` | **F1** | 新增 `approval_is_path_authorization`；`suspension_is_unrecoverable` 的 Approval 判据追加 `options.approvalKind == "path_authorization"`；测试夹具改为路径授权类并新增「非路径审批（3 种形态）⇒ 仍不可恢复」 |
| `crates/pony-agent-core/src/agent/control_plane/mod.rs` | **F2** | 新增纯函数 `approval_is_listable(&PendingControlRequest, now_ms)`（`Pending` + `Approval` + `now_ms <= expires_at_ms`）；`list_pending_approvals` 用 `ask_dispatcher.clock().now_ms()` 过滤；`cancel_approval_request` 前置 `request_kind == Approval` 闸门（`approval_kind_mismatch`）；新增 `mod wave65_r2_approval_tests`（2 例） |
| `crates/pony-agent-core/src/bin/tui/protocol.rs` | **F3** | `PermissionRequest` 新增 `approval_path: Option<String>`（`#[serde(default)]`，机械同步） |
| `crates/pony-agent-core/src/bin/tui.rs` | **F3 / F4** | `permission_request_from_pending` 只从 `options.path` 取授权路径（**不回退 callId**）；新增 `ApprovalPlan` + 纯函数 `approval_plan(&Value, PermissionChoice)` + `approval_snapshot_hint`；`apply_approval_decision` 改为「`authorize == Some` 才 `authorize_path`，`None` 直接 CAS」；`sync_pending_control_request` 合并顺序改为**审批优先、Ask 在后**；新增 `mod wave65_r2_approval_plan_tests`（3 例） |
| `crates/pony-agent-core/tests/tui_interaction_acceptance.rs`<br>`crates/pony-agent-core/tests/tui_interaction_adversarial.rs` | 机械同步 | 4 处 `PermissionRequest { .. }` 字面量补 `approval_path: None,`（纯字段同步，断言逐字未改；两套回归已重跑全绿） |

## R2-1. 门禁原文（命令 + Exit Code）

### `cargo check -p pony-agent-core --bins`
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.35s
GATE1_EXIT=0
```

### `cargo test -p pony-agent-core --test approval_resume_e2e`（冻结红相，未改动）
```
test t1_host_approval_resume_suspends_outside_workspace_read_and_lists_the_pending_approval ... ok
test t2_approval_then_authorized_continuation_turn_completes_the_read ... ok
test t3_cancel_approval_request_is_terminal_and_approve_after_cancel_fails_closed ... ok
test t4_default_runtime_fails_closed_with_approval_resume_unavailable ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.52s
GATE2_EXIT=0
```
逐题：T1 ✅（路径授权类审批仍可恢复）；T2 ✅（批准后续轮放行）；T3 ✅（cancel 终态 + CAS 不放松）；
T4 ✅（能力位默认关 ⇒ fail-fast `approval_resume_unavailable`）。

### `cargo test -p pony-agent-core --bin pony`（含 R2 新单测）
```
test wave65_r2_approval_plan_tests::approval_plan_authorizes_only_path_approvals_on_approval ... ok
test wave65_r2_approval_plan_tests::approval_plan_never_authorizes_on_deny ... ok
test wave65_r2_approval_plan_tests::permission_request_takes_approval_path_only_from_structured_options ... ok
test wave65_approval_unit_tests::approval_continuation_prompt_is_non_empty_and_distinct ... ok
test wave65_approval_unit_tests::apply_approval_decision_without_pending_handle_returns_none ... ok
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
GATE3_EXIT=0
```

### `cargo test -p pony-agent-core --lib`（基线 1316，R2 新增 3 例）
```
test result: ok. 1319 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 45.92s
GATE4_EXIT=0
```
新增：`wave65_r2_approval_tests::approval_is_listable_covers_valid_expired_interaction_and_terminal_states`、
`wave65_r2_approval_tests::cancel_approval_request_rejects_interaction_request`、
`w1_approval_unrecoverable_tests::approval_suspension_unrecoverable_for_non_path_authorization_when_capability_enabled`。
（`turn_control` 专项 = 9 passed / 0 failed。）

### `cargo test -p pony-agent-core --test ask_resume_e2e --test permission_fail_closed_contract`
```
test result: ok. 8 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.73s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.53s
GATE5_EXIT=0
```

### 附加（因 `PermissionRequest` 字段机械同步而触碰的两套回归）
```
cargo test -p pony-agent-core --test tui_interaction_acceptance --test tui_interaction_adversarial
test result: ok. 30 passed; 0 failed; 0 ignored   (acceptance)
test result: ok. 23 passed; 0 failed; 0 ignored   (adversarial)
EXTRA_EXIT=0
```

## R2-2. 逐项结论

- **F1（范围收紧）**：可恢复判据 = `host_approval_resume && session_id.is_some() && approvalKind == "path_authorization"`。
  非路径审批三形态（`approvalKind:"approval"` / 无 `options` / `approvalKind` 非字符串）均 `unrecoverable == true`（单测逐条断言），
  `turn:suspended` 不出、`turn:failed` 以 `approval_resume_unavailable` 收口（T4 佐证默认关路径；非路径形态由单测覆盖）。
- **F2（TTL 活锁 + kind 闸门）**：`approval_is_listable` 四态单测覆盖（有效 / 边界 `now == expires` 仍可列 / 过期不可列 /
  Interaction 不可列 / 非 Pending 三态不可列）；`cancel_approval_request` 对 Interaction 返回 `approval_kind_mismatch`
  且**不消费**（请求仍 Pending、版本未变）。
- **F3（无 path 审批永不批 + 假绿）**：`approval_plan` 是唯一决策口径（`apply_approval_decision` 生产路径也走它，
  非仅测试调用）；有 path+批准 ⇒ `authorize == Some(path)`；无 path+批准 ⇒ `None`（跳过授权直接 CAS）；
  拒绝 ⇒ 恒 `None`（绝不顺手放行被拒路径）。`permission_request_from_pending` 的 `approval_path` 只取 `options.path`，
  `callId` 仅留在**展示**字段 `command_or_path`（单测同时断言两者，防再次假绿）。
- **F4（展示优先序）**：`sync_pending_control_request` 合并顺序 = 审批项 → Ask 项（契约 C4 原文）。
- **已知验收盲区**（契约 §2 C5 R2 登记，不在本波修）：TUI「授权→批准→续轮」正向路径无自动化 e2e
  （`apply_approval_decision` 在 bin 内，集成测试不可导入；bin 单测无法在不跑真实 turn 的前提下造出 Pending Approval）。
  核心语义由 `approval_resume_e2e.rs` T1~T4 覆盖，接线分支由上述纯函数单测覆盖；剩余风险 = backlog F-2。
- **未改动**：冻结测试 `tests/approval_resume_e2e.rs`、并行会话的 `tests/tui_terminal_restore.rs`、`bin/tui.rs` 内
  thinking-bottom-gap 区域、`src-tauri/**`；无 git 写操作。

---

# R3 收据（第二轮审查：1 条新 BLOCKING + 2 条 ADVISORY 转修）

> 契约：`.dev-team/contract-matrix-approval-resume.md` §2 C1/C2/C3/C4 的 R3 增补
> 实施者：Executor（wave-65 task-6）｜时间：2026-11-06

## R3-0. 修改文件与落点

| 文件 | 修复项 | 落点 |
| --- | --- | --- |
| `crates/pony-agent-core/src/agent/dispatcher.rs` | **F1** | 新增 `PATH_PERMISSION_DECISION_SOURCE` + `approval_kind_from_decision_source`；`approval_options_from_arguments` / `approval_prompt_from_arguments` 改按 **`decision_source`** 派生（调用点传入 `ready.decision.decision_source`）；非路径成因**不带 `path` 键**、文案不再谎报「工作区外路径」；新增 `mod wave65_r3_approval_kind_tests`（3 例） |
| `crates/pony-agent-core/src/agent/tool_runtime.rs` | **F2** | `PendingControlRequest::is_path_authorization_approval()`（共享判据：kind == Approval && `options.approvalKind == "path_authorization"`） |
| `crates/pony-agent-core/src/agent/runtime/turn_control.rs` | **F2** | 删除本地复刻判据，`suspension_is_unrecoverable` 改用共享方法；文档同步 |
| `crates/pony-agent-core/src/agent/control_plane/mod.rs` | **F2** | `approval_is_listable` 改用共享方法 ⇒ **列表只列路径授权类**（+ Pending + 未过期）；新增单测「非路径 Approval ⇒ 不可列」（含反向对照） |
| `crates/pony-agent-core/src/bin/tui.rs` | **F3** | 新增 `APPROVAL_STALE_NOTICE` + 纯函数 `stale_pending_notice(handle)`；`close_stale_pending_request` 与 `report_control_request_error` 按句柄 `is_approval` 选文案；新增 `mod wave65_r3_approval_stale_notice_tests`（2 例） |

## R3-1. 门禁原文（命令 + Exit Code）

```
=== cargo check -p pony-agent-core --bins ===
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.47s
GATE1_EXIT=0

=== cargo test -p pony-agent-core --test approval_resume_e2e ===（冻结红相，未改动）
test t1_host_approval_resume_suspends_outside_workspace_read_and_lists_the_pending_approval ... ok
test t2_approval_then_authorized_continuation_turn_completes_the_read ... ok
test t3_cancel_approval_request_is_terminal_and_approve_after_cancel_fails_closed ... ok
test t4_default_runtime_fails_closed_with_approval_resume_unavailable ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.01s
GATE2_EXIT=0

=== cargo test -p pony-agent-core --bin pony ===
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
GATE3_EXIT=0

=== cargo test -p pony-agent-core --lib ===（基线 1319，R3 新增 4 例）
test result: ok. 1323 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 47.22s
GATE4_EXIT=0

=== cargo test -p pony-agent-core --test ask_resume_e2e --test permission_fail_closed_contract ===
test result: ok. 8 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.65s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.47s
GATE5_EXIT=0
```

## R3-2. 逐项结论

- **F1（BLOCKING，成因派生）**：`approvalKind` 现仅由 `decision_source` 决定 ——
  `path_permission_requires_authorization` ⇒ `path_authorization`（携带**原始** `path` + `scope:"read"`），
  其余成因（`tool_rule_ask` / `external_requires_approval` / `plan_execution_approval_gate` / 未知 / 空串）
  一律 `approval` 且**不带 `path` 键**。单测三条：路径成因+path ⇒ path_authorization；四类非路径成因+path ⇒ approval；
  路径成因但无 path ⇒ fail-closed 降级为 approval（文案同步降级，不再声称「工作区外路径」，与 `approvalKind` 不矛盾）。
  由此消除两条旧缺陷路径：① 规则类审批被误标可恢复导致「批准→续轮→再审批」；② path 不存在时 `authorize_path` 必败、approve 永不执行。
- **F2（共享判据 + 列表收紧）**：`PendingControlRequest::is_path_authorization_approval()` 是唯一判据，
  runtime（可恢复性）与 control plane（可列性）同源复用，彻底消除「两处各自解析 options」的漂移面；
  `list_pending_approvals` 现只列路径授权类 + Pending + 未过期三者齐备的请求，非路径审批在 fail-fast 后**不再弹面板**。
- **F3（审批专属失效文案）**：`APPROVAL_STALE_NOTICE = "该审批已过期，本轮不会自动继续；请重新发起该操作"`；
  句柄为审批时 `close_stale_pending_request` / `report_control_request_error` 均用它，Ask 侧文案逐字不变
  （无句柄的演示面板仍走 Ask 文案）。单测同时断言两条文案互不相同且审批文案含「不会自动继续 / 重新发起」。
- **未改动**：冻结测试 `tests/approval_resume_e2e.rs`、并行会话的 `tests/tui_terminal_restore.rs`、
  `bin/tui.rs` 内 thinking-bottom-gap 区域、`src-tauri/**`；无 git 写操作；未做任何 fmt 改写（整仓 fmt 基线即红，同 R1/R2 记录）。
