# Wave-15 红相收据 — L2-AT 对抗破坏（C4）

- 角色：L2-AT（无状态 Subagent 计算池，对抗破坏测试者）
- 交付物：`crates/pony-agent-core/tests/ask_pending_adversarial.rs`（480 行，唯一新增写入的业务侧文件）
- 契约依据：`.dev-team/contracts/wave-15-ask-cas.md` §0（根因）+ §C4（冻结）
- 写域合规：`git status --porcelain -- crates/pony-agent-core/src` → 0 行（**未触碰任何 src/ 业务代码**）；未修改任何其他测试文件
- 时间：2026-10-16T08:35+08:00

## 1. 门禁命令（真实运行，逐字）

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test ask_pending_adversarial --target-dir target-test
```

**EXIT 码：101**（编译 0 warning / 0 error；4 个用例全部 FAILED）

```
running 4 tests
test test_c4_3_expired_ask_fails_closed_then_expiry_reconciles_the_list ... FAILED
test test_c4_2_replay_after_consumption_must_fail_and_must_not_resurrect ... FAILED
test test_c4_1_concurrent_answer_and_cancel_on_same_ask_has_single_winner ... FAILED
test test_c4_4_bulk_200_mixed_terminal_states_list_projection_is_exact ... FAILED

test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

error: test failed, to rerun pass `--test ask_pending_adversarial`
```

## 2. 失败断言原文（SUT 行为面证据）

| 用例 | 位置 | 失败原文 |
| :--- | :--- | :--- |
| C4-1 | `ask_pending_adversarial.rs:198` | `after the single winner consumed the request, list_pending_asks must be empty, but it still lists ["control-000000000000"]` |
| C4-2 | `ask_pending_adversarial.rs:264` | `a consumed ask must never reappear in list_pending_asks (resurrection), but the list contains control-000000000000` |
| C4-3 | `ask_pending_adversarial.rs:347` | `after expire_control_requests, list_pending_asks must be empty, but it still lists ["control-000000000000"]` |
| C4-4 | `ask_pending_adversarial.rs:447` | `list_pending_asks must not list terminal requests; over-listed 100 id(s), e.g. ["control-000000000000", "control-000000000002", "control-000000000004"]` |

四条失败全部指向同一根因（契约 §0）：`ask_control::filter_interaction_requests` 只按
`request_kind == Interaction` 过滤，未按 `state == Pending` 过滤，而
`GovernedDispatcher::pending_requests()` 返回存储中全部控制请求（含 Consumed / Cancelled / Expired）。
**无一条失败来自编译错误、mock 基建异常或符号缺失。**

## 3. 逐条 C4-x 红相状态

| 用例 | 红相状态 | 结论 |
| :--- | :--- | :--- |
| **C4-1** 并发竞态（确定性） | **FAILED（红）** | 8 线程（`Barrier` 同步释放，4×answer_ask + 4×cancel_ask，授权全部由同一 pending 快照 `for_request` 派生）→ 恰好 1 Ok / 7 Err、终态唯一、version == 2 **全部通过**；仅「事后 `list_pending_asks` 为空」失败。 |
| **C4-2** 重放 | **FAILED（红）** | 消费后版本重放 + 陈旧版本重放均 Err、状态/version 未被改写 **通过**；仅「不得复活该请求」失败（该请求从未从列表移除）。 |
| **C4-3** 时钟超期注入 | **FAILED（红）** | `FakeClock` 推进到 `expires_at_ms + 1` 后 answer/cancel 双 Err、`state` 仍 `Pending`、version 仍 1、`expire_control_requests` 返回 1 且状态转 `Expired` **全部通过**；仅「对账后 `list_pending_asks` 为空」失败。 |
| **C4-4** 体量/边界 | **FAILED（红）** | 200 个 Ask（各自唯一 call_id）基线可见 200；偶数序号 100 个 answer/cancel 交替消费成功；消费后存储仍 200 条、仍 Pending 恰 100 **通过**；仅集合相等失败 → over-listed 100（= 已消费集合）、missing 0。证明缺陷是**多列终态**，不是漏列待答请求。 |

### 3.1 非列表不变量隔离证明（排除假红）

为证明各用例中「除列表投影外的断言均真实成立」（即红点精确落在 SUT 列表投影，而非被前置断言遮挡），
在临时副本 `crates/pony-agent-core/tests/zz_probe_adversarial.rs` 中将列表断言替换为探针输出后运行，
逐条打印底层不变量（该副本**已删除**，冻结交付物未被改动）：

```
PROBE c4_1 list_len=1 stored=Cancelled version=2
PROBE c4_2 list_len=1 list_contains_id=true version=2
PROBE c4_3 list_len=1 reconciled=Expired expired_count=1
PROBE c4_4 over=100 missing=0 listed=200 expected=100 store=200
test result: ok. 4 passed; 0 failed
```

⇒ C4-1 单胜者与 version 只自增一次（1→2）成立；C4-2 重放 fail-closed 成立且未改状态；
C4-3 超期 fail-closed / 不经对账不改状态 成立；C4-4 存储未被删除（投影层过滤）成立。
唯一不成立的就是契约要求修复的列表投影。

## 4. 「raise/assert 归属 SUT 栈」判定

判定方法：`RUST_BACKTRACE=1` 重跑并检查栈帧归属，逐条核对断言取值来源。

- **raise 归属**：四条 panic 点均位于**测试文件**（`ask_pending_adversarial.rs:198/264/347/447`）。
  测试只是断言受体，SUT 在这些路径上**不 panic 也不返回 mock 异常**——`list_pending_asks`
  正常返回 `Ok` 值，失败由断言判定。因此判定为：**raise 属测试栈，assert 受体属 SUT 行为面**，
  无任何 mock/基建异常参与（栈帧中不含 `ask_control` / `dispatcher` 的 panic 点）。
- **assert 归属（关键）**：所有被断言的取值 100% 来自 SUT：
  - `ask_control::list_pending_asks(&dispatcher)`（契约 C4 的直接被测入口）；
  - `GovernedDispatcher::pending_request(id)` / `pending_requests()`（CAS 后的真实存储快照）；
  - `ask_control::answer_ask` / `cancel_ask` 的 `Result`（真实 CAS 判定）；
  - `GovernedDispatcher::expire_control_requests(now)` 的返回计数与状态迁移。
- **未使用**：sleep / 概率等待、测试自造 pending 项、任何私有 API 或 mock dispatcher；
  并发同步仅用 `std::sync::{Arc, Barrier}`（确定性、无 sleep）。
- **结论**：红相归属 SUT 契约缺口（`filter_interaction_requests` 缺 `state == Pending` 过滤），
  **非用例缺陷**。

## 5. 门禁复核清单

- [x] 门禁命令真实运行，EXIT 码 101 留档
- [x] 4/4 用例 FAILED，失败原文逐字留档
- [x] 断言全部打在 SUT 行为面（`list_pending_asks` / dispatcher CAS 与存储快照）
- [x] 未修改任何 `src/` 业务代码（`git status` 复核为 0）
- [x] 未修改其他测试文件；临时探针副本已删除
