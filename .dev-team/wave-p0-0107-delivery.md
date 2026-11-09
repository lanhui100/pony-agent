# wave-p0-0107 交付说明（ADR 0107 P0 止血 · P0.1 整改版）

分支 `wave/p0-0107-log-first`（worktree `/home/dm/pony-agent-worktrees/wave-p0-0107`，基线 `c4a5bd81`）
提交链：`6946b8cd`（红相）→ `4604fa77`（v1，**已作废**）→ `5da43513`/`6941882f`（整改契约 v2）→ `929eafef`（P0.1，二次复核判 FAIL）→ `c81f8607`（P0.2）

> ⚠️ v1 被 L3 反审判 FAIL；P0.1 被二次复核再判 FAIL（p0 测试因 `storage_read_only()` 落在 private `mod commit` 而**根本不编译**、stale-epoch 命令在校验前已写进真源）。本文件描述的是 **P0.2 之后**的状态。

## 零、P0.2 追加修复

| 问题 | 修复 | 凭证 |
|---|---|---|
| 验收测试编译失败（2×E0599） | `storage_read_only()` 迁到 `sqlite_session.rs` 公开 `impl SqliteSessionBackend`；`commit` 模块不再定义同名方法 | `--no-run` exit 0；p0 套件 8/8 |
| 被拒命令污染真源 | 写日志**前** `commit::precheck_stale_epoch` 预检（`epoch < current ⇒ StaleEpoch`、不写日志）+ 事务内校验双检（竞态由后者兜底） | 新用例 `c8_stale_epoch_batch_never_reaches_the_session_log`（日志字节 before==after、无被拒消息） |
| R3 已知代价（量化） | 降级实例每批重开 writer 会 `scan_committed_tail` 全扫：10MB ≈ 16.8ms、线性 ⇒ 100MB ≈ 168ms/批 | 二次复核实测；P1 改为内存持有 next_seq + 空闲超时释放 |

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

## 六、P0.3–P0.5（三轮独立复核后的加固）

| 轮次 | 复核发现 | 修复 | 收据 |
|---|---|---|---|
| v3 | 预检 `.ok()?` fail-open；事务拒 stale 不回滚日志 | 预检 fail-closed（读失败⇒不写日志、整批 Failed、busy_timeout=5000）+ `LogAppendReceipt` + 持锁回滚 | c9 新增；lib 1406→1408 |
| v4 | 回滚按过期偏移截断会切掉他人字节（F1，实测 634→265） | `post_append_len` 双检：抢锁后长度不符 ⇒ 不截断、打 `stale_epoch_log_rollback_skipped` | 新单测 `rollback_is_skipped_when_the_file_grew_after_append`；独立复核者 repro 复跑 `foreign_bytes_survived=true`、`expected=452 actual=644` |

最终门禁：`--lib` **1408 passed** / exit 0；p0 **C1–C9 9 passed** / exit 0；`session_log_*` 四套件 + `pa174` exit 0；`check --all-targets` 0 warning；16 个既有套件全绿（`session_row_heal` 为 pre-existing）。

**残余风险（全部 ADVISORY，已登记 P1/P2）**
1. 回滚抢不到日志锁时放弃截断 ⇒ stale 批可能残留（有 `stale_epoch_log_rollback_skipped` 可观测，无数据损坏）；
2. 降级实例每批重开 writer 全扫会话日志（10MB≈16.8ms，线性）；
3. 预检读失败（BUSY/IO）⇒ 该批整轮失败（fail-loud，非静默）；
4. 同会话两实例并发写仍是一方失败；索引滞后无 UI 提示；
5. `persist_events` 仍把 `LogPersisted` 当非成功（与改动前一致，无回归）。

**决策记录**：`.agents/notes/implemented/bug-fix/2026-11-09-log-first-durability-and-index-lag.md`（ADR verify 通过）。
