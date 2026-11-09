# wave-p0-0107 交付说明（ADR 0107 P0 止血）

分支：`wave/p0-0107-log-first`（隔离 worktree `/home/dm/pony-agent-worktrees/wave-p0-0107`，基线 `c4a5bd81`）
提交：`6946b8cd`（红相锚定）→ `4604fa77`（实现）

## 交付内容（对应 ADR 0107 §5 P0）

| 契约 | 改动 | 落点 |
|---|---|---|
| C1 日志先行 | 删除只读短路：抢不到库锁的降级实例仍写会话 JSONL | `agent/sqlite_session_commit.rs::append_session_log_to_file` |
| C2 失败域隔离 | 新增 `PersistCommandOutcome::LogPersisted`；日志已落盘后的索引失败不再整轮回滚，内存 live 照常提交 | `agent/session/backend.rs`、`agent/sqlite_session.rs::persist_commands_batch`、`agent/session/store.rs::append_turn_fallible` |
| C3 日志失败仍 fail-loud | 日志写失败 ⇒ `Failed`（SQLite 一行不改、内存不动） | 语义保持不变 |
| C4 可写路径零回归 | `Succeeded`/`Unsupported`/`StaleEpoch` 原样透传 | — |

观测：`index_deferred: session=… outcome=LogPersisted`（可 grep）。

## 物理收据

| 门禁 | 结果 | 出处 |
|---|---|---|
| `p0_log_first_durability`（C1–C4） | 红相 exit **101**（C1/C2 FAIL、C3/C4 PASS）→ 绿相 exit **0**（4/4） | `.dev-team/red-phase-p0-0107.log` / `.dev-team/green-phase-p0-0107.log` |
| `session_log_wiring` + `session_read_from_log` + `session_log_store` + `session_event_carriers` | exit **0**（27 tests） | 同上 + Lead 复跑（6+6+7+8 全绿） |
| `pa174_history_wiring_and_followup_contract` | exit **0**（13 tests） | green-phase log |
| `cargo check --all-targets` | exit **0**（0 warning） | green-phase log |

## 安全性证据（并发写）

只读实例恢复写 JSONL **不会**造成同一会话并发写坏日志：`SessionLogStore::open_write` 对会话日志文件施加**每会话排他 flock**（`log_store.rs:886` `try_lock_exclusive`），抢不到者返回 `Locked` → 重试后失败 → `append_session_log_to_file` 返回 `Err` → 整批 `Failed`（fail-loud，而不是静默或写坏）。
⇒ P0 的并发单元是**会话**；同一会话真正并发写的排队/租约属于 P1。

## 已知限制与后续

1. 同一会话被两个进程同时使用时，后开者本轮会**明确失败**（不丢数据、不假绿）；P1 会话级写者租约 + UI 提示解决。
2. 索引滞后期间，侧边栏/检索可能不是最新（索引可由日志重建）；P1 增加 `projection_lag` 观测。
3. `store.rs` 两处单命令路径（trace 投影缓存回写、`flush_turn_events`）仍把 `LogPersisted` 当“非成功”处理（偏保守，fail-loud），未纳入本次写域，登记为 P1 顺手项。
4. P1（会话级租约 + SQLite 多进程串行写）、P2（存储/租约抽象、远程 agent）按 ADR 0107 分层推进。

## 合并方式

```
cd /home/dm/pony-agent
git merge --no-ff wave/p0-0107-log-first
```
注意：主工作区当前有他人未提交改动，合并前请确认无交叉文件（本次仅 4 个 `src/agent/**` 文件 + 1 个新测试文件）。

## L3 对抗复核（独立）

verdict = **PASS**，0 BLOCKING / 4 ADVISORY（`/tmp/diag/review-p0-0107.json`）：

1. 并发写日志：**VERIFIED**——`log_store.rs::open_write` 在锁内 `try_lock`（flock LOCK_EX|NB），后到者 `Locked` 失败，不会交错写坏。
   ADVISORY：只读实例长期持有该会话的 `LogWriter`（flock）时，主实例再写**同一会话**会 `Locked → Failed → 内存回滚`（反向 C2 症状）——需要“两实例同写同会话”，属 P1 会话级租约要解决的问题。
2. 误判掩盖丢数据：**REFUTED**——`append_turn` 批必含带 `session_id` 的命令，`grouped.is_empty()` 仅 `RemoveSession`/元数据-only 批可触发，生产不可达。
3. 映射过宽：**VERIFIED**——`delete_session_logs_after_commit` 仍只看原始 `Succeeded`，且在映射之前调用。
4. 未覆盖消费方：**RISK（非丢数据）**——`persist_events` 把 `LogPersisted` 当 false ⇒ 保留 buffer 重试、重复 `FlushEvents` 行；该行为与改动前的 `Failed` 路径**完全一致**（无回归），登记为 P1 顺手项。

（复核时间盒内未复跑既有套件；Lead 已独立复跑 `session_log_*` 四套件 6+6+7+8 全绿。）
