# Wave-16 契约矩阵：ask 面板「终态请求被重新列出 → CAS 报错」修复

状态：**冻结（Lead）**。Test Agent 依此写红相；Executor 只读，不得改本文件与验收测试。

## 0. 症状与根因（Lead 只读侦察结论，红相负责证实）

用户症状：在 ask 面板选择选项后点击「回答 / 取消」，弹出

```
取消失败：control request 'control-0000000000' cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch
```

因果链（代码级）：

1. `GovernedDispatcher::pending_requests()`（`crates/pony-agent-core/src/agent/dispatcher.rs:626`）
   返回**存储中全部**控制请求，包含 `Consumed` / `Cancelled` / `Expired` 终态。
2. `ask_control::filter_interaction_requests()`（`.../agent/ask_control.rs:23`）**只按
   `request_kind == Interaction` 过滤，未按 `state == Pending` 过滤** → 已消费的 Ask
   仍被 `list_pending_asks` / `ask_list_pending` 当作待答请求列出。
3. `AskPanel.vue` 每 2.5s `refresh()` 轮询 → 用户已应答消费的卡片以 `version=2` 重新出现，
   且轮询因列表非空永不停止。
4. 用户再次点「回答 / 取消」：`require_current_version(pending.version=2, expected=2)` 预检
   **通过**，随后 dispatcher CAS 因 `state != Pending` 拒绝，落到通用文案
   `cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch`
   （`dispatcher.rs:1265-1274`）。
   → 提交前 `answer()` 的 `pendingAsks.find()` 守卫无效，因为该条目正是宿主列出的。

次生同文案路径：TTL（`DispatchBudgetConfig::control_request_expiry_ms` = 60_000ms，
`budget.rs:74`）已过但 `state` 仍为 `Pending`（全仓 `ask_expire` 无调用点）→
`can_consume` 的 `now_ms <= expires_at_ms` 失败，报同一文案。

## 1. 验收契约（冻结）

### C1 后端列表契约（Rust，`ask_control::list_pending_asks`）

- **C1-1**：仅返回 `request_kind == Interaction` **且** `state == Pending` 的请求。
- **C1-2**：`answer_ask` 消费成功后，同一 dispatcher 的 `list_pending_asks` **不再包含**
  该 `request_id`（当前实现为包含 → 红相必须失败）。
- **C1-3**：`cancel_ask` 取消成功后，列表**不再包含**该 `request_id`。
- **C1-4**：`expire_control_requests(now > expires_at_ms)` 之后，列表**不再包含**该
  `request_id`。
- **C1-5**：多个 Ask 混合终态时，列表精确等于「仍 `Pending` 且 `Interaction`」集合
  （不得多列、不得漏列仍在等待的请求）。
- **C1-6**：`HostControlPlane::list_pending_asks(None)` / `(Some(session))` 投影同样满足
  C1-2~C1-5（Tauri 命令 `ask_list_pending` 的用户可见面）。

### C2 前端过期守卫契约（TypeScript，`src/stores/ask.ts` / `src/components/AskPanel.vue`）

- **C2-1**：本地已过期（`expiresAtMs > 0 && expiresAtMs <= now`）的 Ask，调用
  `askStore.answer()` / `cancel()` **不得**发出 `ask_answer` / `ask_cancel` IPC
  （杜绝注定失败的 CAS）。
- **C2-2**：上述调用必须触发对账（`ask_expire` → `ask_list_pending`），使列表移除该请求，
  并给出明确中文提示（含「过期」），不得再出现裸 `cannot be consumed` 文案。
- **C2-3**：未过期 Ask 的 `answer` / `cancel` 行为不变（仍以列表 `version` 作 CAS 提交）。
- **C2-4**：`AskPanel` 对已过期 Ask 禁用选项 / 回答 / 取消按钮（视觉上不可再触发 C1 报错）。

### C3 不变量（禁止回归）

- **C3-1**：`PendingControlRequest::can_consume` 的 CAS 语义**不得放宽**：重放、过期、
  跨会话、版本/nonce 不匹配、终态重复消费仍必须 fail-closed 报错。
- **C3-2**：非 `Interaction`（`Approval`）请求不得出现在 `list_pending_asks`。
- **C3-3**：`graph_list_ask_waits` / `graph_resume_ask` 绑定的 `expectedVersion` 语义不变。

### C4 对抗破坏（L2-AT，`crates/pony-agent-core/tests/ask_pending_adversarial.rs`）

- **C4-1（并发竞态，确定性）**：N 线程同时对同一 Ask 发起 `answer_ask` / `cancel_ask`，
  恰有一个 `Ok`，其余 `Err`；事后列表为空，终态唯一，`version` 只自增一次。
- **C4-2（重放）**：以消费后的版本重复提交必须 `Err`，且列表**不得**复活该请求。
- **C4-3（时钟超期注入）**：`FakeClock` 推进到 `expires_at_ms + 1` 后，`answer_ask` /
  `cancel_ask` 必须 fail-closed 且不改状态；执行 `expire_control_requests` 后列表清空。
- **C4-4（体量/边界）**：批量（≥200）请求中混合终态，列表精确等于 Pending 集合。

## 2. 写域与责任（正交，advisory 锁）

| 角色 | write_scopes | 交付 |
| :--- | :--- | :--- |
| test-agent（L2-T） | `crates/pony-agent-core/tests/ask_pending_contract.rs`、`tests/ask-pending-expiry.spec.ts`、`tests/ask-panel-expired.spec.ts` | 红相验收测试 + 失败收据 |
| L2-AT（Subagent 池） | `crates/pony-agent-core/tests/ask_pending_adversarial.rs` | C4 破坏用例 + 红相收据 |
| executor-core | `crates/pony-agent-core/src/agent/ask_control.rs` | C1 修复（最小闭包） |
| executor-ui | `src/stores/ask.ts`、`src/components/AskPanel.vue` | C2 修复（最小闭包） |

**红线**：Executor 不得修改验收测试（冻结后只读）；不得改动 `can_consume` 语义（C3-1）；
不得夹带无关重构。

## 3. 机器门禁（物理收据，Exit Code 0 方可放行）

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_contract --target-dir target-test
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_adversarial --target-dir target-test
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts tests/ask-store.spec.ts
npm run typecheck
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_user_acceptance --target-dir target-test   # 既有 ask 验收不回归
```

红相阶段：C1-2~C1-6 / C4 对应用例必须失败（`EXIT != 0`），失败原文留档
`.dev-team/wave15-red-receipts.md`。

## 4. 红相收据（Wave-16，Lead 复核；真实 EXIT 码）

| 门禁 | 命令 | 结果 |
| :--- | :--- | :--- |
| L2-T Rust | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_contract --target-dir target-test` | **EXIT=101**，0 passed / 6 failed |
| L2-T Web | `npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts` | **EXIT=1**，4 failed / 4 passed（C2-3 对照通过） |
| L2-AT Rust | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_adversarial --target-dir target-test` | **EXIT=101**，0 passed / 4 failed |

- 逐字证据（与用户症状同源）：`{"requestId":"control-000000000000","requestKind":"interaction","state":"consumed","version":2,...}`
  仍被 `ask_list_pending` 投影返回（`c1_6_host_control_plane_projection_excludes_terminal_requests`）。
- L2-AT 隔离证明：并发/重放/超期/体量四条用例中，非列表不变量全部通过
  （8 线程竞态恰 1 Ok/7 Err、`version==2`、重放 Err、超期 fail-closed 且 state 仍 `Pending`、
  存储仍 200 条），失败点精确落在列表投影层 `filter_interaction_requests` 缺少 `state == Pending`。
- 归属判定：失败断言受体均为 SUT 行为（`list_pending_asks` / dispatcher CAS / store IPC 面），
  非测试基建缺陷；无 `#[ignore]`、无裸 pass、无吞异常。
