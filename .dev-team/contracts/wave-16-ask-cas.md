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

## 5. 契约修订（Lead 一票裁定，Wave-16 收口前）

**修订 1（C2 过期边界口径，采纳 Executor 证据）**：C2-1 原文写的 `expiresAtMs <= now` 与宿主
`PendingControlRequest::can_consume` 的放行条件 `now_ms <= expires_at_ms`
（crates/pony-agent-core/src/agent/tool_runtime.rs:135-150）不等价：`<=` 会把**宿主仍会消费**的
`expiresAtMs == now` 请求判为过期，静默丢弃用户答案。现冻结口径为：

- **C2-5（新）**：本地过期判定 = `expiresAtMs > 0 && expiresAtMs < now`（严格小于），即仅当
  `now > expiresAtMs` 才不可提交；`expiresAtMs === now` 仍视为可提交；`expiresAtMs <= 0` 表示无 TTL。
- 证据：Executor 探针 300 次，`<=` 误判 2–6 次（0.7–2%），`<` 为 0/300（探针已删除）。
  该口径同时是 `src/stores/ask.ts:isAskExpired` 与 `AskPanel` 渲染时钟的唯一真值。

**修订 2（测试自身缺陷加固，不计入熔断）**：`tests/ask-pending-expiry.spec.ts:233` 的对照夹具
`expiresAtMs: Date.now() + 1` 余量仅 1ms，负载下可能越过 TTL 造成 flaky。由测试作者（test-agent）
将余量放大到 `Date.now() + 60_000`，并补 C2-5 边界断言（`expiresAtMs === now` 非过期 /
`expiresAtMs === now - 1` 过期）。实施者（Executor）对冻结测试仍只读。

## 6. 契约修订 3（L3 审查 B 发现，Lead 裁定；新增 C2-6）

**审查反例（已由 Lead 物理复核）**：`AskPanel` 对过期 Ask 禁用全部控件（AskPanel.vue:156/168/173/183）
后，唯一会调用 `ask_expire` 的入口（`store.answer()/cancel()` 的过期守卫 → `reconcileExpiredAsk`
→ `expire()`）在 UI 上**不可达**；宿主侧又无任何自动 expire 调用点
（`grep -rn "ask_expire" src/ src-tauri/src/` 仅命中 `src/stores/ask.ts:299` 与本波新增守卫）。
后果：TTL 一过，卡片永久滞留（既不能答也不能取消），提示「正在同步最新待处理列表」永不兑现，
挂起 run 永久停在 `WaitingUser`。→ 原 C2-2 的「必须对账」在 UI 上失效，判定 **FAIL**，必须补正。

**C2-6（新，冻结）**：`askStore.refresh()`（轮询 `schedulePoll` 与事件监听共用的唯一列表入口）
在列表快照中发现**本地已过期**的 Ask 时，必须**自动对账**：

1. IPC 序列必须为 `ask_list_pending` → `ask_expire` → `ask_list_pending`（**单次**重列，
   禁止递归调用 `refresh()`、禁止同一次 refresh 内重复 `ask_expire`，防止 IPC 风暴）；
2. 重列后 `pendingAsks` 不得再包含已过期请求（宿主已将其转为 `Expired`，被 C1 的
   `state == Pending` 过滤剔除）；
3. `ask_expire` 失败不得被吞掉（保留 `this.error` 的失败文案）且不得阻止本次重列；
4. 未过期快照**不得**产生额外的 `ask_expire` IPC（无过期即零副作用，避免每 2.5s 一次无效写）。

**保留项**：C2-1/C2-2 的点击路径守卫保持不变（覆盖 ≤2.5s 轮询窗口内的程序化调用），
但不得再作为唯一对账路径。

## 7. 契约修订 4（L3 审查 A 发现 1/3 + 生产面事实，Lead 裁定）

**Lead 复核的生产面事实**：`AskPanel.vue` 在 `src/` 内**无挂载点**（ADR-0030 已裁定消息流不再渲染
`<AskPanel />`），生产 ask 卡片位于 `src/components/chat/WorkspaceComposer.vue`（:866/913/936/949），
其按钮仅受 `askIsBusy` 约束；store 的 `error` 就是该卡片显示的报错文案（:948）。因此：

- **C2-4（AskPanel 禁用）保留**为组件级契约（测试可挂载），但**不构成生产面防线**；
- **生产面防线统一收敛到 store**（下称 C2-6~C2-10），避免本波次写入
  `WorkspaceComposer.vue`：该文件当前带有**并行 wave-15 会话的未提交改动**，本波次若写入将无法
  安全定向提交（会把他人 WIP 一并入库）。→ 登记遗留项（见 §8），待其 WIP 提交后由
  `dev-backlog` 消费。

**C2-6（修订单飞）**：`refresh()` 发现本地过期 Ask 时自动对账 `ask_list_pending → ask_expire →
ask_list_pending`（单次重列，禁止递归）；**对账必须单飞**：并发触发（双击 / 轮询与点击交叠）
只能产生一次 `ask_expire`。`ask_expire` 失败不得被吞且不阻止重列；快照无过期 Ask 时零副作用。

**C2-7（提交时刻时钟，取代 §2 中 `lastRefreshMs` 口径）**：`answer()`/`cancel()` 的过期判定
必须使用**提交时刻**（`Date.now()`），不得使用快照时钟 `lastRefreshMs`。依据（审查 A finding 1）：
轮询在列表为空时自终止（ask.ts:327-330），之后新 Ask 仅由 `turn:suspended` 事件 refresh 列出，
`lastRefreshMs` 冻结在列出时刻 T；用户等过 `expiresAtMs` 再点击时 `expiresAtMs < T` 为假 →
守卫漏判 → 照发注定失败的 CAS → 重现用户可见的裸文案。严格 `<` 边界（C2-5）不变。

**C2-8（轮询自愈）**：`refresh()` 在快照非空且当前未轮询时必须重启轮询（`startPolling()`），
使 `lastRefreshMs` 持续前进、C2-6 自动对账得以持续驱动；快照为空时行为不变（stopPolling）。

**C2-9（重入与乐观状态保护）**：过期对账分支必须置于忙标志之前置位（或单飞），双击不得产生
2×`ask_expire`；对账重列**不得回填**正被 `answeringRequestId`/`cancellingRequestId` 处理的请求
（保护 §answer/cancel 的乐观移除）。

**C2-10（渲染时钟）**：`AskPanel` 的渲染时钟改为实时 `Date.now()`（每次渲染/轮询重算），
不再使用快照时钟；过期态的可视禁用仍是"展示层"，权威判定在 store（C2-7）。

### 7.1 C2-6 静默性补充（Lead 裁定，避免 error 永久滞留）

`refresh()` 驱动的**自动**对账成功后**不得**在 `store.error` 留下用户可见文案
（否则轮询随后因列表清空而 `stopPolling`，该文案会永久滞留在卡片上）；仅当 `ask_expire` /
重列**失败**时保留具体失败文案。点击路径（C2-2）仍必须给出含「过期」的显式提示。

## 8. 遗留项（登记 `management/task-system/01_TASK_BOARD.md`，不在本波次实施）

| 编号 | 事项 | 不实施原因 |
| :--- | :--- | :--- |
| `PA-120` | 生产面 composer 卡片（`src/components/chat/WorkspaceComposer.vue`）过期态禁用对齐 | 该文件携带并行 wave-15 会话未提交 WIP，写入无法安全定向提交；本波次以 store 兜底（提交时刻判定 + 自动对账 ≤2.5s 移出卡片） |
| `PA-121` | `ask_expire(now_ms)` 允许渲染层自传时钟（全局、跨 session、kind 无关） | L3 审查 B 判定不构成新权限提升（同层可直接 `ask_answer`），按最小闭包不夹带；已登记安全加固条款 |

## 10. 二轮 L3 复审缺陷收口与裁定（Lead）

二轮独立复审产出 3 条 finding：
1. **B2 `tests/ask-pending-expiry.spec.ts:506` C2-9 断言未约束 SUT**：用例使用 `mockExpiredAskHost` + `await store.refresh()` 提前清空快照，导致两次点击在 `if (!listed) return null;` 返回，未覆盖单飞分支。
   - **收口**：改为直接注入快照 `store.pendingAsks = [expired]; store.lastRefreshMs = Date.now()` 并 `tauriMocks.mockSafeInvoke.mockClear()`。变异验证确认单飞分支若被删除则 `expireCalls` 为 2 导致失败，断言真实有效。
2. **A2 `src/stores/ask.ts:330` 静默对账在飞时点击路径早退遗漏提示**：
   - **收口**：`reconcileExpiredAsk()` 改为 `await this.reconcileExpiredAsks({ surfaceError: true }); if (!this.error) this.error = ASK_EXPIRED_MESSAGE; return null;`，即便并发单飞早退也保证点击路径留下明确过期提示。
3. **A2 `src/components/chat/WorkspaceComposer.vue:955` error 宿主在 activeAsk 卸载时同批次卸载**：
   - **收口**：该文件携带并行 wave-15 会话 WIP，同源归入 `PA-120` 遗留项登记并在其验收条款中显式约束「error 提示宿主与 activeAsk 存活解耦」。

