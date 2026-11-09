# wave-p0-0107 交付说明（ADR 0107 P0 止血 · P0.1 整改版）

分支 `wave/p0-0107-log-first`（worktree `/home/dm/pony-agent-worktrees/wave-p0-0107`，基线 `c4a5bd81`）
提交链：`6946b8cd`（红相）→ `4604fa77`（v1，**已作废**）→ `5da43513`/`6941882f`（整改契约）→ `929eafef`（P0.1）

> ⚠️ v1 被 L3 反审判 FAIL（stale-epoch 被吞成 `LogPersisted`、漏跑 `--lib` 2 条断言红、降级实例长期持 flock 饿死主实例、dry-run 零写入失效、零字节批判错）。本文件描述的是 **P0.1 整改后**的状态。

## 一、行为契约（P0 目标）

| ID | 契约 | 落点 |
|---|---|---|
| C1 | 锁降级实例（`is_read_only && !storage_read_only`）仍写会话 JSONL | `sqlite_session_commit.rs::append_session_log_to_file` |
| C2 | 日志已落盘 + 索引失败 ⇒ `LogPersisted`：内存 live 照常提交，索引滞后 | `backend.rs`、`sqlite_session.rs::persist_commands_batch`、`store.rs::append_turn_fallible` |
| C3 | 日志写失败 ⇒ `Failed`（内存不动、SQLite 零改动） | 同上 |
| C4 | 可写实例零回归（`Succeeded/Unsupported/StaleEpoch` 原样透传） | — |
| R1 | stale-epoch 类型化：`ApplyFailure::StaleEpoch` ⇒ `StaleEpoch`，**不得**被改判 `LogPersisted`（内存不前进） | `sqlite_session_commit.rs:891/956`、`sqlite_session.rs:1141` |
| R3 | 降级实例 writer **按批开/放**（append 后立即释放 flock），不饿死主实例 | `log_sink.rs::append_commands_releasing` |
| R4 | 维护句柄（`new_read_only`/`--dry-run`）仍零写入（连日志都不写） | `append_session_log_to_file` 的 `storage_read_only` 短路 |
| R5 | 零字节批（`RemoveSession`-only）不得判 `LogPersisted` | `log_sink.append_commands -> Result<bool>` + `index_failure_after_log(outcome, log_written)` |
| R6 | `ReplaceAll` 路径的索引失败可观测（`index_deferred`） | `fold.rs:2001` 透出 outcome、`store.rs:2606` |

## 二、物理收据（`929eafef`）

| 门禁 | 结果 |
|---|---|
| `cargo test --lib` | exit 0，**1405 passed**（Lead 独立复跑同结果） |
| `p0_log_first_durability`（C1–C4 + c5/c6/c7） | exit 0，7 用例 |
| `session_log_wiring`/`session_read_from_log`/`session_log_store`/`session_event_carriers`/`pa174…` | 全 exit 0 |
| `cargo check --all-targets` | exit 0（0 warning） |
| 26 个既有套件加跑 | 仅 `session_row_heal::resume_recovers…` 失败，已用 `git archive` 在 `6946b8cd`/`6941882f` 双基线对照判定 **pre-existing**（非本次引入） |

原文：`.dev-team/red-phase-p0-0107.log`、`.dev-team/green-phase-p0-0107.log`（含已声明语义变更 3 条及其理由）。

## 三、安全性与已知代价

- **并发写安全**：会话日志有每会话排他 flock（`log_store.rs:886`），后到者 `Locked` 失败，不会交错写坏。
- **已知代价（R3）**：降级实例每批 `open_writer → scan_committed_tail` 会**从头扫该会话日志**（`log_store.rs:625`），大日志下每轮多花「扫描一遍文件」的时间。仅影响锁降级实例（罕见路径），换取主实例不被饿死。P1 应改为「内存持有 next_seq + 空闲超时释放」或会话级租约。
- **仍未覆盖**：同会话两实例并发写仍是一方失败（fail-loud）；索引滞后期间列表/检索可能不新鲜；`persist_events` 对 `LogPersisted` 仍返回 false（与改动前 `Failed` 行为一致，无回归）。

## 四、后续（P1/P2，见 ADR 0107）

P1：会话级写者租约（epoch fencing）+ SQLite 多进程串行写（WAL/busy_timeout/BUSY 重试）+ `projection_lag` 观测。
P2：`SessionLogStore`/`SessionIndexStore` 抽象 + `Lease` 抽象 + 远程 agent 真源归属。

## 五、合并

```
cd /home/dm/pony-agent
git merge --no-ff wave/p0-0107-log-first
```
主工作区当前有他人未提交改动（`sqlite_session.rs`/`sqlite_session_commit.rs`/`store.rs` 同文件），合并前需先处理。
