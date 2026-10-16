# Wave-16 绿相收据（物理，原生 EXIT 码）

契约：`.dev-team/contracts/wave-16-ask-cas.md` §C1（冻结，只读）。
各责任方只追加/替换自身章节，禁止整文件覆盖。

## executor-core

- 任务：task-2（wave-15 fix(core)：`ask_list_pending` 仅返回 Pending Interaction）
- 唯一变更文件：`crates/pony-agent-core/src/agent/ask_control.rs`
  - `filter_interaction_requests` 谓词由 `request_kind == Interaction` 收窄为
    `request_kind == Interaction && state == PendingControlRequestState::Pending`；
    `list_pending_asks` 继续组合它；同步修正两条文档注释的 "pending" 语义
    （= 仍在等待宿主答复，而非存储中全部条目）。
  - `#[cfg(test)]`：既有 8 条断言全部保留，新增
    `filter_pending_asks_drops_terminal_interaction_requests`（真实断言：Consumed /
    Cancelled / Expired 的 Interaction 与终态 Approval 均不被保留，仅 Pending Interaction 保留）。
- 边界：未触碰 `dispatcher.rs` / `tool_runtime.rs` / 任何验收测试；`can_consume` CAS 语义零改动；
  无无关重构/格式化；无 `#[ignore]`、无吞异常。`git status --porcelain` 中 Rust 侧仅本文件被修改。

### 门禁 1 — L2-T Rust 契约

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_contract --target-dir target-test
```

```
running 6 tests
test c1_1_list_pending_asks_only_exposes_pending_interaction_requests ... ok
test c1_2_answered_ask_is_no_longer_listed ... ok
test c1_3_cancelled_ask_is_no_longer_listed ... ok
test c1_4_expired_ask_is_no_longer_listed ... ok
test c1_5_mixed_terminal_states_list_exactly_the_pending_set ... ok
test c1_6_host_control_plane_projection_excludes_terminal_requests ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

**EXIT=0**（6 passed / 0 failed；编译 0 warning）

### 门禁 2 — 既有 ask 验收不回归

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_user_acceptance --target-dir target-test
```

```
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

**EXIT=0**（9 passed / 0 failed）

### 门禁 3 — 模块单测

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --lib agent::ask_control --target-dir target-test
```

```
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1014 filtered out; finished in 0.00s
```

**EXIT=0**（9 passed / 0 failed，含新增 1 条）

### 附加 — L2-AT 对抗用例（契约 §3 门禁，一并核验）

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_adversarial --target-dir target-test
```

```
test test_c4_1_concurrent_answer_and_cancel_on_same_ask_has_single_winner ... ok
test test_c4_2_replay_after_consumption_must_fail_and_must_not_resurrect ... ok
test test_c4_3_expired_ask_fails_closed_then_expiry_reconciles_the_list ... ok
test test_c4_4_bulk_200_mixed_terminal_states_list_projection_is_exact ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

**EXIT=0**（4 passed / 0 failed）

warning 计数核验：`cargo test ... --test ask_pending_contract ... | grep -ci warning` → `0`。

## executor-ui

- 任务：task-3（wave-15 fix(ui)：过期 ask 禁止提交 + 面板禁用），契约 `.dev-team/contracts/wave-16-ask-cas.md` §C2。
- 唯一变更文件：`src/stores/ask.ts`、`src/components/AskPanel.vue`（冻结验收测试 `tests/ask-pending-expiry.spec.ts` / `tests/ask-panel-expired.spec.ts` 零改动，`git status --porcelain -- tests/` 无这两项）。
- 实现：
  - `ask.ts`：新增 `ASK_EXPIRED_MESSAGE`、`isAskExpired(ask, nowMs)`；`answer()`/`cancel()` 在 `listed` 快照上判定本地过期后**不发** `ask_answer`/`ask_cancel`，改走新增 action `reconcileExpiredAsk()`（`expire()` = `ask_expire` → `refresh()`，随后设置含「过期」提示，返回 `null`）。未过期路径仍以 `{requestId, expectedVersion: listed.version, version: listed.version}` 提交。
  - `AskPanel.vue`：补 `lastRefreshMs` 到 `storeToRefs`，`renderNowMs()` 以 `lastRefreshMs` 为渲染时钟（缺失回退 `Date.now()`，无新增定时器/依赖）；新增 `askIsExpired`/`askIsLocked`；过期时禁用选项按钮、`ask-answer-submit`、`ask-cancel-submit`、输入框，并显示含「过期」的 `[data-testid="ask-expired-hint"]`。既有 `data-testid` 与无障碍属性未变。

### 边界口径（Lead 请复核，非静默偏离）

- 契约 C2-1 写 `expiresAtMs <= now`；实现取**严格** `expiresAtMs < nowMs`。依据：宿主 `can_consume` 的放行条件是 `now_ms <= expires_at_ms`，即 `expiresAtMs == nowMs` 时 CAS **仍会成功**；用 `<=` 会抢先禁掉一个仍可消费的请求、静默丢弃用户答案，且与冻结对照用例
  `C2-3: an ask expiring only later is not treated as expired at submit time`（`expiresAtMs = Date.now() + 1` 后经历 `refresh()` + `answer()`）冲突。物理证据：同一探针循环 300 次，`<=` 口径误判过期 2–6 次（0.7%–2%），`<` 口径 0/300。
- 时钟取列表快照 `lastRefreshMs`（`refresh()` 成功后打点，轮询每 2.5s 重打），与面板渲染同一时钟；不用提交瞬间 `Date.now()`：前端只能从快照判定 TTL，且改口径后 `<` + 提交瞬间时钟亦可通过对照用例，但会使 UI 与动作口径不一致。

### 门禁 1 — 前端验收 + 回归（原生 EXIT）

```bash
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts tests/ask-store.spec.ts tests/AskPlanPanel.spec.ts tests/workspace-composer-ask.spec.ts
```

```
 ✓ tests/ask-pending-expiry.spec.ts (5 tests) 29ms
 ✓ tests/ask-store.spec.ts (17 tests) 53ms
 ✓ tests/ask-panel-expired.spec.ts (3 tests) 120ms
 ✓ tests/AskPlanPanel.spec.ts (7 tests) 183ms
 ✓ tests/workspace-composer-ask.spec.ts (7 tests) 188ms

 Test Files  5 passed (5)
      Tests  39 passed (39)
```

**EXIT=0**（红相基线：同两文件 `EXIT=1`，4 failed / 4 passed）

### 门禁 2 — 类型

```bash
npm run typecheck
```

```
> pony-agent@0.1.146 typecheck
> vue-tsc --noEmit
```

**EXIT=0**（0 error；无 `[TYPECHECK EXTERNAL-CHURN]` —— wave-15 并行 WIP 文件本次未产生类型错误）

### 门禁 3 — 边界稳定性（防 flaky，非契约门禁）

```bash
for i in $(seq 1 15); do npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts; done
```

```
REPEATS=15 FAILURES=0
```

### 门禁 4 — 轮询驱动过期翻转（临时探针，跑后已删除，不入库）

`vi.useFakeTimers()` 下挂载 `AskPanel`，Ask 的 TTL 为 +3000ms：初次挂载与第 1 个 poll tick 后
回答/取消按钮 `disabled === undefined`（仍走 CAS）；第 2 个 tick 越过 TTL 后 `lastRefreshMs` 重打点，
按钮全部 `disabled` 且 `[data-testid="ask-expired-hint"]` 出现。

```
POLL_FLIP_OK lastRefreshMs=1792111846169 expiresAtMs=1792111844169
 ✓ (1 test) | EXIT=0
```

## executor-ui2（二轮 C2-6~C2-10）

- 任务：wave-16 二轮前端修复（接手前一位 Executor 未完成项）。契约 `.dev-team/contracts/wave-16-ask-cas.md`
  §6/§7/§7.1（只读）。冻结核验测试全程只读、零改动。
- 唯一变更文件：`src/stores/ask.ts`、`src/components/AskPanel.vue`。未触碰 `src/components/chat/WorkspaceComposer.vue`
  与 `src/components/HomeWorkspace.vue`（mtime 08:20:54，早于本会话 09:14；并行 WIP 未受影响）、Rust 代码、
  任何测试；无任何 git 写操作。
- 实现（按 Lead 规格逐条）：
  - `ask.ts:37` 模块级 `listPendingAsks(sessionId?)`（`args.sessionId` 仅非空时下发，非数组回落 `[]`）；
    `ask.ts:64-65/101-102` 新增 `pollingDesired` / `reconcilingExpired` 状态。
  - `ask.ts:131-157` `refresh()` 改用 `listPendingAsks`（`:142`），C2-6 静默自动对账（`:144-146`）、
    C2-8 轮询自愈（`:147-149`），`loading/error/lastRefreshMs` 语义不变。
  - `ask.ts:164-169` `filterInFlight()`（剔除 `answeringRequestId`/`cancellingRequestId` 条目，C2-9b）。
  - `ask.ts:326-356` `reconcileExpiredAsks({sessionId?, surfaceError?})`：同步置位单飞（`:330-333`）→
    `ask_expire`（失败留「过期处理失败…」且继续）→ **单次**重列 + `filterInFlight`（`:342-343`，不调用 `refresh()`，无递归）→
    仅 `surfaceError && !error` 时置 `ASK_EXPIRED_MESSAGE`（§7.1 静默）；`ask.ts:363` `reconcileExpiredAsk()` 收敛为
    `await this.reconcileExpiredAsks({ surfaceError: true }); return null;`。
  - `ask.ts:195/282` `answer()`/`cancel()` 过期判定改为提交时刻 `isAskExpired(listed)`（C2-7），CAS 双字段与 stale 分支不变。
  - `ask.ts:390-425` 轮询三态：`startPolling` 先置 `pollingDesired`；新增 `haltPolling()`（不清 desired）；
    tick 空列表改调 `haltPolling()`；`stopPolling()` = `pollingDesired=false` + `haltPolling()`；`ask.ts:460-461` `reset()` 复位两标志。
  - `AskPanel.vue:11-12/22-24` 渲染时钟改实时 `Date.now()`，`storeToRefs` 移除 `lastRefreshMs`；`data-testid` 与模板结构未变。
  - 边界：保留 `expire()`（`tests/ask-store.spec.ts:350` 仍调用）；二轮新路径不再经 `expire()`/`refresh()`。

### 门禁 1 — 前端验收 + 回归（原生 EXIT，**裁决前基线**：test-agent 修订测试机制之前）

```bash
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts tests/ask-store.spec.ts tests/AskPlanPanel.spec.ts tests/workspace-composer-ask.spec.ts
```

```
 ✓ tests/ask-store.spec.ts (17 tests)
 ✓ tests/AskPlanPanel.spec.ts (7 tests)
 ✓ tests/workspace-composer-ask.spec.ts (7 tests)
 ❯ tests/ask-panel-expired.spec.ts (4 tests | 1 failed)
 ❯ tests/ask-pending-expiry.spec.ts (14 tests | 3 failed)
 Test Files  2 failed | 3 passed (5)
      Tests  4 failed | 45 passed (49)
```

**EXIT=1**（既有 3 文件零回归；4 条失败全部落在冻结的两份核验文件，见下节裁决请求）

### 门禁 2 — 类型

```bash
npm run typecheck
```

**EXIT=0**（0 error）

### 门禁 3 — 边界稳定性（15 次，**裁决前基线**）

```bash
for i in $(seq 1 15); do npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts; done
```

```
run 1..15: EXIT=1 | Tests 4 failed | 14 passed (18)   （15/15 完全一致，无 flaky）
```

### 未通过 4 条（裁决前）：物理证据 → 已由 Lead 裁决 Option A 解除

| # | 断言 | 物理原因 | 是否任何实现可满足 |
| :-- | :-- | :-- | :-- |
| 1 | `ask-pending-expiry.spec.ts:100`（C2-1 cancel） | 该用例（`:92-97`）与 C2-6 用例（`:300-305`）**夹具逐字节等价**（同一 `mockExpiredAskHost`、同一 `expiresAtMs = Date.now()-1000`、同样单次 `await store.refresh()`），却要求 `pendingAsks == ["ask-1"]`；C2-6 要求同一调用后 `pendingAsks == []`。**逻辑互斥**。 | **否** |
| 2 | `ask-pending-expiry.spec.ts:146`（C2-1 answer） | refresh 已按 C2-6/§7.1 静默对账并移除该请求 → `answer()` 的 `!listed` 分支按契约「其余逻辑保持」直接 `return null` → `store.error` 为 `null`。 | 仅当新增「未列出但本地已过期 → 置显式过期提示」分支（超出本轮规格） |
| 3 | `ask-pending-expiry.spec.ts:507`（C2-9b，5s 超时） | 该用例 `ask_expire` 返回 **deferred** promise，且 `resolveExpire(...)` 在 `await store.refresh()`（`:546`）**之后**才调用；C2-6/§7.1 要求 refresh `await` 自动对账 → refresh 永久阻塞。 | 仅当 refresh 以有界等待（超时/竞态）返回——与 §7.1「单次重列后返回」口径冲突（不推荐） |
| 4 | `ask-panel-expired.spec.ts:187`（C2-10） | 组件无响应式依赖变化时 `await wrapper.vm.$nextTick()` **不会触发重渲染**；`vi.setSystemTime` 不触发任何定时器。临时探针（已删除）实测：`Date.now()` 增量 6000（Date 确被 fake）、`$nextTick()` 后 `attributes("disabled") === undefined`、`$forceUpdate()` 后 `=== ""`（禁用生效）。即 src 侧已正确，缺的是用例触发重渲染。 | **否** |

裁决：**Option A**（Lead 裁定，src 维持本轮规格实现；4 处用例自身机制缺陷由 test-agent 修订）——
`ask-pending-expiry.spec.ts:100`/`:146` 与 `C2-9b` 改为**直接注入快照** `store.pendingAsks = [...]`
（绕开 C2-6 自动对账与 deferred `ask_expire` 的阻塞），`ask-panel-expired.spec.ts:187` 补显式
`wrapper.vm.$forceUpdate()` 重渲染触发。`src/**` 在裁决前后**零改动**。

### 收口 — 裁决后复测（原生 EXIT，全绿）

```bash
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts tests/ask-store.spec.ts tests/AskPlanPanel.spec.ts tests/workspace-composer-ask.spec.ts
```

```
 ✓ tests/ask-pending-expiry.spec.ts (14 tests)
 ✓ tests/ask-store.spec.ts (17 tests)
 ✓ tests/ask-panel-expired.spec.ts (4 tests)
 ✓ tests/AskPlanPanel.spec.ts (7 tests)
 ✓ tests/workspace-composer-ask.spec.ts (7 tests)
 Test Files  5 passed (5)
      Tests  49 passed (49)
```

**EXIT=0**（目标 18 条 spec 用例全部 passed；既有 3 文件 31 条零回归）

```bash
npm run typecheck   # EXIT=0，0 error
for i in $(seq 1 15); do npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts; done
```

```
run 1..15: EXIT=0 FAILURES=0   （15/15 一致，无 flaky）
```

**Lead 裁决后按原命令重跑（最终收口，09:33 本机）**：门禁① **EXIT=0 / 49 passed (49)**
（18 条 spec 全绿 + 既有 31 条零回归）；门禁② **EXIT=0 / 0 error**；门禁③ **15/15 EXIT=0 FAILURES=0**。
额外核验：仓库内其余引用 ask store / AskPanel 的 spec 一并复跑
`tests/acceptance/stage-1-ask-user.spec.ts`（14 passed）与 `tests/workspace-width-alignment.spec.ts`
（2 passed），**EXIT=0 / 16 passed**，无回归。src 冻结（`md5 eb11d0a21e03cfe531c670dc9f2928c6` ask.ts / `5cfb4c2686c31d6341160f324cf2ac18` AskPanel.vue，
mtime 09:15，全部绿色复测期间未再写入）。
