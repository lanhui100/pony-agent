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
