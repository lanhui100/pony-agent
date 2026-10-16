# Wave-15 红相收据 — ask 待答列表终态过滤 + 本地过期守卫（task-1 / test-agent）

契约依据：`.dev-team/contracts/wave-15-ask-cas.md`（C1-1 ~ C1-6、C2-1 ~ C2-4，冻结）。
执行环境：Linux 原生 shell，工作目录 `/home/dm/pony-agent`；无 Windows 脚本参与。
交付测试（仅测试文件，未改动任何 `src/**` 业务代码）：

- `crates/pony-agent-core/tests/ask_pending_contract.rs`（C1-1 ~ C1-6）
- `tests/ask-pending-expiry.spec.ts`（C2-1 ~ C2-3）
- `tests/ask-panel-expired.spec.ts`（C2-4）

分级判定：**L2（组件与契约测试）**；涉及 `PendingControlRequest` 状态机与前端 IPC 守卫，
用例全部使用真实 dispatcher store / 真实 ask_user descriptor / 真实 AskPanel 挂载，
并含对照用例（未过期仍提交、无 TTL 仍提交、未过期按钮可用）防止"过宽修复"。

---

## 1. Rust 契约门禁（C1）

命令（原文）：

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_contract --target-dir target-test
```

**EXIT = 101**（红相必须失败）

```
running 6 tests
test c1_1_list_pending_asks_only_exposes_pending_interaction_requests ... FAILED
test c1_2_answered_ask_is_no_longer_listed ... FAILED
test c1_3_cancelled_ask_is_no_longer_listed ... FAILED
test c1_4_expired_ask_is_no_longer_listed ... FAILED
test c1_5_mixed_terminal_states_list_exactly_the_pending_set ... FAILED
test c1_6_host_control_plane_projection_excludes_terminal_requests ... FAILED

failures:

---- c1_1_list_pending_asks_only_exposes_pending_interaction_requests stdout ----
thread 'c1_1_...' panicked at crates/pony-agent-core/tests/ask_pending_contract.rs:166:9:
assertion `left == right` failed: list_pending_asks must never expose terminal request `control-000000000000` (state=Consumed)
  left: Consumed
 right: Pending

---- c1_2_answered_ask_is_no_longer_listed stdout ----
thread 'c1_2_...' panicked at crates/pony-agent-core/tests/ask_pending_contract.rs:205:5:
a consumed Ask must not be re-listed as pending (this is the user-visible CAS trap)

---- c1_3_cancelled_ask_is_no_longer_listed stdout ----
thread 'c1_3_...' panicked at crates/pony-agent-core/tests/ask_pending_contract.rs:232:5:
a cancelled Ask must not be re-listed as pending

---- c1_4_expired_ask_is_no_longer_listed stdout ----
thread 'c1_4_...' panicked at crates/pony-agent-core/tests/ask_pending_contract.rs:263:5:
an expired Ask must not be re-listed as pending

---- c1_5_mixed_terminal_states_list_exactly_the_pending_set stdout ----
thread 'c1_5_...' panicked at crates/pony-agent-core/tests/ask_pending_contract.rs:332:5:
assertion `left == right` failed: the list must be exactly the still-pending Interaction set — no terminal leaks, no missing waits
  left: {"control-000000000000", "control-000000000001", "control-000000000002", "control-000000000003"}
 right: {"control-000000000003"}

---- c1_6_host_control_plane_projection_excludes_terminal_requests stdout ----
thread 'c1_6_...' panicked at crates/pony-agent-core/tests/ask_pending_contract.rs:369:9:
assertion `left == right` failed: projected ask must be pending: {"callId":"call-1","descriptorId":"builtin:ask_user","descriptorSnapshotId":"builtin-tool-catalog-v1","expiresAtMs":61000,"finalArgsDigest":"effee2edf64e09a3","nonce":"nonce-000000000001","options":["是","否"],"policyDigest":"6f84ed1b500e5b56","prompt":"已答？","requestId":"control-000000000000","requestKind":"interaction","runId":"run-1","sessionId":"session-1","state":"consumed","turnId":"turn-1","version":2}
  left: Some("consumed")
 right: Some("pending")

test result: FAILED. 0 passed; 6 failed; 0 ignored; 0 measured; 0 filtered out
error: test failed, to rerun pass `--test ask_pending_contract`
```

红相定位证据：C1-6 投影原文中 `"state":"consumed"` 的请求仍然出现在
`HostControlPlane::list_pending_asks` 返回的数组里 —— 与用户症状（已消费 Ask 以
`version=2` 复活 → 再提交触发 CAS 通用报错）逐字对应。

编译状态：`cargo test` 编译阶段 **0 warning / 0 error**，文件只引用既有 `pub` API
（`ToolRegistrySnapshot::builtin`、`GovernedDispatcher`、`ask_control::{answer_ask,cancel_ask,list_pending_asks}`、
`HostControlPlaneBuilder::ask_dispatcher`），不存在"因符号不存在而失败"的伪红相。

对照基线（未被本 wave 改动、红相下仍全绿）：
`cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_user_acceptance --target-dir target-test`
→ **EXIT = 0**，`9 passed; 0 failed`。

---

## 2. 前端门禁（C2）

命令（原文）：

```bash
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts
```

**EXIT = 1**（红相必须失败）

```
 ❯ tests/ask-pending-expiry.spec.ts (5 tests | 2 failed) 37ms
     × C2-1/C2-2: cancel never submits an already-expired ask and reconciles via ask_expire 22ms
     × C2-1/C2-2: answer never submits an already-expired ask and reconciles via ask_expire 5ms
     ✓ C2-1: a zero/absent TTL is treated as non-expiring, not as expired 3ms
     ✓ C2-3: a live ask still cancels and answers with the version observed at listing time 3ms
     ✓ C2-3: an ask expiring only later is not treated as expired at submit time 2ms
 ❯ tests/ask-panel-expired.spec.ts (3 tests | 2 failed) 95ms
     × C2-4: disables the option, answer, and cancel controls for a locally expired ask 57ms
     ✓ C2-4 (control): a live ask keeps its controls enabled 19ms
     × C2-4: an expired ask with no options still disables answer/cancel 18ms

 Test Files  2 failed (2)
      Tests  4 failed | 4 passed (8)
```

失败原文（关键行）：

```
 FAIL  tests/ask-panel-expired.spec.ts > AskPanel — expired ask guard > C2-4: disables the option, answer, and cancel controls for a locally expired ask
AssertionError: expected undefined to be defined
 ❯ tests/ask-panel-expired.spec.ts:89:45
     89|       expect(button.attributes("disabled")).toBeDefined();

 FAIL  tests/ask-panel-expired.spec.ts > AskPanel — expired ask guard > C2-4: an expired ask with no options still disables answer/cancel
AssertionError: expected undefined to be defined
 ❯ tests/ask-panel-expired.spec.ts:139:85
    139|     expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes…

 FAIL  tests/ask-pending-expiry.spec.ts > ask store — local expiry guard > C2-1/C2-2: cancel never submits an already-expired ask and reconciles via ask_expire
AssertionError: expected "vi.fn()" to not be called with arguments: [ 'ask_cancel', Anything ]
  2nd vi.fn() call: [ "ask_cancel", { "expectedVersion": 2, "requestId": "ask-1", "version": 2 } ]
 ❯ tests/ask-pending-expiry.spec.ts:95:43

 FAIL  tests/ask-pending-expiry.spec.ts > ask store — local expiry guard > C2-1/C2-2: answer never submits an already-expired ask and reconciles via ask_expire
AssertionError: expected "vi.fn()" to not be called with arguments: [ 'ask_answer', Anything ]
  2nd vi.fn() call: [ "ask_answer", { "answer": "x", "expectedVersion": 2, "requestId": "ask-1", "version": 2 } ]
 ❯ tests/ask-pending-expiry.spec.ts:125:43
```

---

## 3. 失败归属判定

**上述失败全部归属 SUT（业务代码），而非测试基建缺陷**：全部三个文件编译/加载成功、无导入或
API 名错误，失败点均为对真实业务行为的断言 —— `ask_control` 列表未按 `state == Pending` 过滤，
以及 `src/stores/ask.ts` / `src/components/AskPanel.vue` 缺少本地过期守卫；红相期间曾出现两处
测试自身瑕疵（`builtin:ask_user` schema 要求 `description` 导致 dispatch 无控制结果、
fixture 中 `options: null` 被默认值覆盖），均已按"修正测试本身"处理并复跑确认，当前日志中
不再存在任何来自测试基建的错误。

## 4. 契约红相状态表

| 契约 | 用例 | 红相 | 证据 |
| :--- | :--- | :--- | :--- |
| C1-1 | `c1_1_list_pending_asks_only_exposes_pending_interaction_requests` | FAILED | 列表暴露 `state=Consumed` |
| C1-2 | `c1_2_answered_ask_is_no_longer_listed` | FAILED | 消费后仍被列出 |
| C1-3 | `c1_3_cancelled_ask_is_no_longer_listed` | FAILED | 取消后仍被列出 |
| C1-4 | `c1_4_expired_ask_is_no_longer_listed` | FAILED | 过期后仍被列出 |
| C1-5 | `c1_5_mixed_terminal_states_list_exactly_the_pending_set` | FAILED | 期望 `{...003}`，实得 4 个 id 全集 |
| C1-6 | `c1_6_host_control_plane_projection_excludes_terminal_requests` | FAILED | 投影含 `"state":"consumed"` |
| C2-1 | store `cancel`/`answer` 过期用例 | FAILED | 仍发出 `ask_cancel` / `ask_answer` |
| C2-2 | 同上（`ask_expire` + 对账 + 「过期」提示） | FAILED | 未调用 `ask_expire`，`store.error` 为空 |
| C2-3 | store 未过期用例（2 条） | PASSED（对照，不得回归） | 仍以 listed.version 提交 |
| C2-4 | `AskPanel` 过期禁用（2 条） | FAILED | 选项/回答/取消均未 disabled |

测试数据与环境：全部用例使用内存 mock / 进程内 dispatcher，无临时文件残留；唯一外部访问是
`HostControlPlaneBuilder::build()` 默认 runtime 打印的
`[pony-agent][session] loading sessions from SQLite /home/dm/.local/share/PonyAgent/sessions.db`（产品默认只读加载，
与 crate 内既有 `ask_control_plane_surface_*` 用例同源），测试不写入该库。

---

# § 收口加固修订（Wave-16 §5 修订 2，task-1 reopen，test-agent）

范围：仅测试文件；`src/**` 与 `crates/**/src/**` 全程只读（`git status` 证实本 agent 未触碰）。
契约依据：`.dev-team/contracts/wave-16-ask-cas.md` §5（修订 1 = C2-5 严格小于口径；修订 2 = 夹具加固）。

## A. 改动清单

| # | file:line | 改动 |
| :--- | :--- | :--- |
| 1 | `tests/ask-pending-expiry.spec.ts:233`（改前）→ `:230-234`（改后） | 对照夹具 `expiresAtMs: Date.now() + 1` → `Date.now() + 60_000`（原 1ms 余量在负载下会越过 TTL，属构造性 flaky）；断言语义不变，仍是"未过期必须走 `ask_answer` CAS 提交" |
| 2 | `tests/ask-pending-expiry.spec.ts:3` | `import { isAskExpired, useAskStore } from "@/stores/ask";` |
| 3 | `tests/ask-pending-expiry.spec.ts:261-285` | 新增 `C2-5: isAskExpired mirrors the host boundary ...`（固定时钟 `now = 1_700_000_000_000`，10 条边界断言） |

C2-5 断言明细（固定时钟，无负载依赖）：
`(expiresAtMs=now, now) === false`（等于边界仍可提交，与宿主 `can_consume` 的 `now_ms <= expires_at_ms` 等价）·
`(now-1, now) === true` · `(now, now+1) === true` · `(now+1, now) === false` ·
`(0, now) === false` · `(-1, now) === false` · `(0, Date.now()) === false`（0 = 无 TTL）·
`(null, now) === false` / `(undefined, now) === false`（退化输入不抛错）。

## B. 门禁（真实原生命令 + EXIT 码）

命令（原文）：

```bash
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts
```

**EXIT = 0**

```
 ✓ tests/ask-pending-expiry.spec.ts (6 tests) 23ms
 ✓ tests/ask-panel-expired.spec.ts (3 tests) 88ms

 Test Files  2 passed (2)
      Tests  9 passed (9)
   Duration  1.31s
```

（用例数由 8 → 9：新增 C2-5；C2-1/C2-2 两条过期守卫、C2-3 两条对照、C2-4 三条面板用例全部转绿。）

## C. 15 次连跑稳定性统计（防 Flaky 凭据）

```
run 01 EXIT=0 |       Tests  9 passed (9)
run 02 EXIT=0 |       Tests  9 passed (9)
run 03 EXIT=0 |       Tests  9 passed (9)
run 04 EXIT=0 |       Tests  9 passed (9)
run 05 EXIT=0 |       Tests  9 passed (9)
run 06 EXIT=0 |       Tests  9 passed (9)
run 07 EXIT=0 |       Tests  9 passed (9)
run 08 EXIT=0 |       Tests  9 passed (9)
run 09 EXIT=0 |       Tests  9 passed (9)
run 10 EXIT=0 |       Tests  9 passed (9)
run 11 EXIT=0 |       Tests  9 passed (9)
run 12 EXIT=0 |       Tests  9 passed (9)
run 13 EXIT=0 |       Tests  9 passed (9)
run 14 EXIT=0 |       Tests  9 passed (9)
run 15 EXIT=0 |       Tests  9 passed (9)
TOTAL_RUNS=15 FAILURES=0
```

## D. 结论

- 夹具加固后 15/15 全绿、FAILURES=0，C2-3 对照用例不再具备 TTL 越界窗口。
- C2-5 边界由断言直接锁定 `expiresAtMs < now` 严格小于口径（`==` 边界可提交、`0/负数` 视为无 TTL），
  与宿主 `can_consume` 语义一致，未放宽任何既有断言。
- 本次修订未触碰 `src/**`；无 `skip` / `#[ignore]` / 吞异常。

---

# § C2-6 红相（Wave-16 §6 / 契约修订 3，task-1 二次 reopen，test-agent）

契约依据：`.dev-team/contracts/wave-16-ask-cas.md` §6（C2-6 冻结）。
反例：`AskPanel` 对过期 Ask 禁用全部控件后，唯一调用 `ask_expire` 的入口（`answer`/`cancel`
的过期守卫）在 UI 上不可达，宿主无自动 expire → 卡片永久滞留、run 永停 `WaitingUser`。
修正方向：`askStore.refresh()` 在快照中发现本地已过期 Ask 时自动对账。

## A. 新增用例（`tests/ask-pending-expiry.spec.ts`，仅此文件）

| 用例 | 行 | 断言 |
| :--- | :--- | :--- |
| `C2-6: refresh auto-reconciles a locally expired ask (list → expire → list)` | `:296-316` | 命令序列严格等于 `["ask_list_pending","ask_expire","ask_list_pending"]`；`pendingAsks` 为空 |
| `C2-6: a live snapshot produces no extra ask_expire IPC (zero side effects)` | `:318-338` | 命令序列严格等于 `["ask_list_pending"]`（无 `ask_expire`）；快照仍含该请求 |
| `C2-6: a failing ask_expire is surfaced and still re-lists the snapshot` | `:341-371` | `store.error` 非空（不吞异常）；仍完成 `list → expire → list` 且快照为空 |

辅助：`reconcileCommands()`（`:53-57`）按调用顺序过滤出 `ask_list_pending` / `ask_expire`，
使序列断言精确到顺序与次数（可捕获递归 refresh / 重复 expire 的 IPC 风暴）。

## B. 门禁（真实原生命令 + EXIT 码）

```bash
npx vitest run tests/ask-pending-expiry.spec.ts
```

**EXIT = 1**（红相必须失败）

```
 ❯ tests/ask-pending-expiry.spec.ts (9 tests | 2 failed) 38ms
     ✓ C2-1/C2-2: cancel never submits an already-expired ask and reconciles via ask_expire
     ✓ C2-1/C2-2: answer never submits an already-expired ask and reconciles via ask_expire
     ✓ C2-1: a zero/absent TTL is treated as non-expiring, not as expired
     ✓ C2-3: a live ask still cancels and answers with the version observed at listing time
     ✓ C2-3: an ask expiring only later is not treated as expired at submit time
     ✓ C2-5: isAskExpired mirrors the host boundary `expiresAtMs < now` exactly
     × C2-6: refresh auto-reconciles a locally expired ask (list → expire → list) 10ms
     ✓ C2-6: a live snapshot produces no extra ask_expire IPC (zero side effects)
     × C2-6: a failing ask_expire is surfaced and still re-lists the snapshot 3ms

 Test Files  1 failed (1)
      Tests  2 failed | 7 passed (9)
```

失败原文（关键行）：

```
 FAIL  tests/ask-pending-expiry.spec.ts > ask store — local expiry guard > C2-6: refresh auto-reconciles a locally expired ask (list → expire → list)
AssertionError: expected [ 'ask_list_pending' ] to deeply equal [ 'ask_list_pending', …(2) ]

- Expected
+ Received

  [
    "ask_list_pending",
-   "ask_expire",
-   "ask_list_pending",
  ]

 ❯ tests/ask-pending-expiry.spec.ts:312:33

 FAIL  tests/ask-pending-expiry.spec.ts > ask store — local expiry guard > C2-6: a failing ask_expire is surfaced and still re-lists the snapshot
AssertionError: expected null to be truthy
 ❯ tests/ask-pending-expiry.spec.ts:366:25
```

红相定位：`refresh()` 当前只发一次 `ask_list_pending`，完全不对账过期快照（实收序列
`["ask_list_pending"]`）；失败链与审查反例一致 —— 过期卡片被面板禁用后无人再触发
`ask_expire`。C2-6 的对照用例（未过期零副作用）已通过，证明断言未过宽。

## C. 结论

- 新增 3 条 C2-6 用例中 2 条红相失败、1 条对照通过；其余 C2-1/C2-2/C2-3/C2-5 全绿，
  失败仅来自 `refresh` 缺少自动对账（属 SUT 行为，非测试基建缺陷）。
- 本次仅修改 `tests/ask-pending-expiry.spec.ts` 与本章节；`src/**` 全程只读，
  无 `skip`/`#[ignore]`/吞异常，既有断言未放宽。

---

# § C2-7~C2-10 红相（Wave-16 §7 / 契约修订 4，task-1 三次 reopen，test-agent）

契约依据：`.dev-team/contracts/wave-16-ask-cas.md` §7（C2-6 修订单飞、C2-7~C2-10）与 §7.1
（refresh 驱动的静默对账成功后 `store.error` 必须为空）。改动仅限
`tests/ask-pending-expiry.spec.ts` 与 `tests/ask-panel-expired.spec.ts`；`src/**` 全程只读。

## A. 新增/加固用例

| 契约 | 用例 | 行 | 断言要点 |
| :--- | :--- | :--- | :--- |
| §7.1 | C2-6 用例 1 末尾追加 | `ask-pending-expiry.spec.ts:319` | `expect(store.error).toBeFalsy()`（静默对账不留文案） |
| C2-7 | `C2-7: cancel after the TTL must use the submit-time clock, not the listing snapshot` | `:384-416` | fake timers：list 给出 `expiresAtMs = T0+5s`，`advanceTimersByTime(6s)` 后 `cancel` → 无 `ask_cancel`、有 `ask_expire`、`error` 含「过期」 |
| C2-7 | `C2-7: answer after the TTL ...` | `:418-448` | 同上以 `answer` 验：无 `ask_answer`、有 `ask_expire`、`error` 含「过期」 |
| C2-8 | `C2-8: refresh restarts polling when the snapshot becomes non-empty again` | `:450-483` | 轮询因空列表自终止（前置断言 `polling === false`）→ 非空 `refresh()` 后 `polling === true`，且 `advanceTimersByTimeAsync(ASK_POLL_INTERVAL_MS)` 后确有一次新的 `ask_list_pending` |
| C2-9 | `C2-9: two overlapping cancels of one expired ask issue exactly one ask_expire` | `:485-505` | 同步连续两次 `cancel(同一过期 Ask)` → `ask_expire` 恰 1 次、`ask_cancel` 0 次 |
| C2-9b | `C2-9b: a reconcile re-list must not re-populate an optimistically removed ask` | `:507-565` | 可控 deferred 编排：`ask_expire` 与 `ask_answer` 双挂起，期间 `answer(A)` 乐观移除 A 并置忙；resolve `ask_expire` 后 list#2 返回 `[A]` → 重列**不得回填** A |
| C2-10 | `C2-10: the render clock is real time, not the frozen listing snapshot` | `ask-panel-expired.spec.ts:156-196` | fake timers + `vi.setSystemTime(+6s)`（不触发任何定时器，`lastRefreshMs` 保持陈值）→ 回答/取消/选项按钮 `disabled` 且文本含「过期」；辅助 `flushMicrotasks()`（`:54-60`）避免 fake timers 下 `setTimeout` 挂死 |

C2-9b 可确定性复现：不使用 sleep / 概率等待 / 重试，全部由显式 deferred resolve 与时序确定
（本机 5 次连跑结果完全一致，见 §C）。

## B. 门禁（真实原生命令 + EXIT 码）

```bash
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts
```

**EXIT = 1**（红相必须失败）

```
 ❯ tests/ask-pending-expiry.spec.ts (14 tests | 7 failed) 48ms
     ✓ C2-1/C2-2: cancel never submits an already-expired ask and reconciles via ask_expire
     ✓ C2-1/C2-2: answer never submits an already-expired ask and reconciles via ask_expire
     ✓ C2-1: a zero/absent TTL is treated as non-expiring, not as expired
     ✓ C2-3: a live ask still cancels and answers with the version observed at listing time
     ✓ C2-3: an ask expiring only later is not treated as expired at submit time
     ✓ C2-5: isAskExpired mirrors the host boundary `expiresAtMs < now` exactly
     × C2-6: refresh auto-reconciles a locally expired ask (list → expire → list) 7ms
     ✓ C2-6: a live snapshot produces no extra ask_expire IPC (zero side effects) 1ms
     × C2-6: a failing ask_expire is surfaced and still re-lists the snapshot 2ms
     × C2-7: cancel after the TTL must use the submit-time clock, not the listing snapshot 6ms
     × C2-7: answer after the TTL must use the submit-time clock, not the listing snapshot 3ms
     × C2-8: refresh restarts polling when the snapshot becomes non-empty again 3ms
     × C2-9: two overlapping cancels of one expired ask issue exactly one ask_expire 1ms
     × C2-9b: a reconcile re-list must not re-populate an optimistically removed ask 2ms
 ❯ tests/ask-panel-expired.spec.ts (4 tests | 1 failed) 88ms
     ✓ C2-4: disables the option, answer, and cancel controls for a locally expired ask
     ✓ C2-4 (control): a live ask keeps its controls enabled
     ✓ C2-4: an expired ask with no options still disables answer/cancel
     × C2-10: the render clock is real time, not the frozen listing snapshot 12ms

 Test Files  2 failed (2)
      Tests  8 failed | 10 passed (18)
```

失败原文（每条一行，均为新断言）：

```
C2-7  cancel : AssertionError: expected "vi.fn()" to not be called with arguments: [ 'ask_cancel', Anything ]
               （实收 { "expectedVersion": 2, "requestId": "ask-1", "version": 2 }，:411）
C2-7  answer : AssertionError: expected "vi.fn()" to not be called with arguments: [ 'ask_answer', Anything ]
               （实收 { "answer": "x", "expectedVersion": 2, "requestId": "ask-1", "version": 2 }，:443）
C2-8         : AssertionError: expected false to be true（store.polling，:474）
C2-9         : AssertionError: expected 2 to be 1（ask_expire 次数，:503）
C2-9b        : AssertionError: expected [ 'A' ] to not include 'A'（对账重列回填 A，:561）
C2-10        : AssertionError: expected undefined to be defined（ask-answer-submit 无 disabled，:187）
```

红相定位：现行实现仍以 `lastRefreshMs` 快照时钟判过期（C2-7：`ask.ts:154/241`）、
`refresh()` 不重启轮询（C2-8：`ask.ts:104-129`）、过期分支在忙标志之前无单飞（C2-9：
`ask.ts:282-290`）、对账重列无条件覆盖 `pendingAsks`（C2-9b）、`AskPanel` 渲染时钟取
`lastRefreshMs`（C2-10：`AskPanel.vue:22-24`）。

## C. 红相确定性（5 次连跑）

```
run 1 EXIT=1 |       Tests  8 failed | 10 passed (18)
run 2 EXIT=1 |       Tests  8 failed | 10 passed (18)
run 3 EXIT=1 |       Tests  8 failed | 10 passed (18)
run 4 EXIT=1 |       Tests  8 failed | 10 passed (18)
run 5 EXIT=1 |       Tests  8 failed | 10 passed (18)
```

无既有用例回归：C2-1/C2-2/C2-3/C2-5 与 C2-6 零副作用对照、C2-4 三条仍全部通过（10 passed）；
失败的 8 条全部来自本轮新断言与前一轮已登记的 C2-6 红相（用例 1 因 §7.1 追加的
`error` 断言随用例一起红，属预期）。无 `skip`/`#[ignore]`/吞异常，失败均非语法或导入错误。

---

# § 裁决后用例机制修订（Wave-16 §9，task-1 四次 reopen，test-agent）

依据：`.dev-team/contracts/wave-16-ask-cas.md` §9（Lead 复核判定 4 条失败为**用例机制缺陷**，
非实现偏离）。仅改 `tests/ask-pending-expiry.spec.ts` 与 `tests/ask-panel-expired.spec.ts`；
`src/**` 全程只读。**断言强度只增不减**，无 skip/ignore，未删除任何核心断言。

## A. 修订明细（file:line）

| # | 用例 | 行 | 修订（机制） | 保留/新增的断言 |
| :--- | :--- | :--- | :--- | :--- |
| ① | `C2-1/C2-2: cancel ...` | `ask-pending-expiry.spec.ts:90-131`（注入点 `:110`） | 不再 `await store.refresh()`（会被 C2-6 自动对账先清空）；改为直接注入 `store.pendingAsks = [expired]` + `lastRefreshMs`，list mock 一律 `[]` | 保留：无 `ask_cancel`、`ask_expire` 早于最后一次 `ask_list_pending`、`pendingAsks == []`、`error` 含「过期」且不含 `cannot be consumed`；**新增**注入后快照前置断言 |
| ② | `C2-1/C2-2: answer ...` | `ask-pending-expiry.spec.ts:133-168`（注入点 `:150`） | 同 ①（Lead 裁决 ④） | 保留：无 `ask_answer`/`ask_cancel`、顺序、`pendingAsks == []`、`error` 含「过期」 |
| ③ | `C2-9b: ...` | `ask-pending-expiry.spec.ts:528-590`（注入点 `:567`） | 去掉会等待 deferred `ask_expire` 的 `await store.refresh()`（超时根因）；直接注入 `[A(live), B(expired)]` → `answer(A)`（`ask_answer` deferred，乐观移除+置忙）→ `cancel(B)` 驱动对账（`ask_expire` deferred）→ resolve → 重列 `[A]` | 保留：重列不回填 A；**新增** `commands()` 含 `ask_expire` 前置断言、`pendingAsks` 精确 `[]` |
| ④ | `C2-10: ...` | `ask-panel-expired.spec.ts:156-205`（注入点 `:176`） | 直接注入陈旧快照；推进 fake 时钟后补一次显式重渲染 `$forceUpdate()`（组件无响应式变更时不重渲染） | 保留：回答/取消/选项按钮 `disabled`、文本含「过期」；**新增**推进前"未过期可用"前置断言与 `lastRefreshMs < Date.now()`（证明判定取自实时钟而非快照） |

C2-4 三条面板用例（含「未过期对照仍可点」）未改动，继续全绿。

## B. 门禁（真实原生命令 + EXIT 码）

```bash
npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts
```

**EXIT = 0**

```
 ✓ tests/ask-pending-expiry.spec.ts (14 tests) 41ms
 ✓ tests/ask-panel-expired.spec.ts (4 tests) 88ms

 Test Files  2 passed (2)
      Tests  18 passed (18)
   Duration  1.30s
```

## C. 15 次连跑稳定性

```
run 01 EXIT=0 |       Tests  18 passed (18)
run 02 EXIT=0 |       Tests  18 passed (18)
run 03 EXIT=0 |       Tests  18 passed (18)
run 04 EXIT=0 |       Tests  18 passed (18)
run 05 EXIT=0 |       Tests  18 passed (18)
run 06 EXIT=0 |       Tests  18 passed (18)
run 07 EXIT=0 |       Tests  18 passed (18)
run 08 EXIT=0 |       Tests  18 passed (18)
run 09 EXIT=0 |       Tests  18 passed (18)
run 10 EXIT=0 |       Tests  18 passed (18)
run 11 EXIT=0 |       Tests  18 passed (18)
run 12 EXIT=0 |       Tests  18 passed (18)
run 13 EXIT=0 |       Tests  18 passed (18)
run 14 EXIT=0 |       Tests  18 passed (18)
run 15 EXIT=0 |       Tests  18 passed (18)
TOTAL_RUNS=15 FAILURES=0
```

## D. 结论

- 4 条用例机制缺陷已修复（陈旧卡片窗口改为直接注入快照、C2-10 补显式重渲染、C2-9b 改显式
  deferred 编排），门禁转绿：**EXIT=0 / 18 passed**，15 次连跑 **FAILURES=0**。
- 全部核心断言（不发 `ask_answer`/`ask_cancel`、`ask_expire` 恰 1 次、重列不回填在飞请求、
  过期按钮 `disabled`、`error` 含「过期」、C2-6 序列与静默语义）逐条保留，并新增 5 处前置/精确
  断言，断言强度只增不减；无 `skip`/`#[ignore]`/吞异常。
- 本次仅改两个测试文件与本章节；`src/**` 全程只读（实现侧零偏离，与 Lead 裁决一致）。




