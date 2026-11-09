# 契约矩阵：ADR 0107 · P1 波次（wave/p1-session-lease）

基线：`e043f32f`（= P0 分支 HEAD），worktree `/home/dm/pony-agent-worktrees/wave-p1-0107`
真值：`docs/decisions/0107-session-scoped-writer-lease-and-log-first-durability.md` §5 P1

## 不变量

1. **索引多进程可写**：本地文件系统上多个 `pony` 进程可同时对 `sessions.db` 提交短事务，
   由 SQLite WAL + `busy_timeout` + BUSY 重试保证串行化；**不存在**「第二个进程整库只读」。
2. **破坏性维护独占**：`rebuild-index` / `migrate` / `VACUUM` / schema 迁移 等跨会话全库操作
   仍须持有库级独占锁；抢不到 ⇒ 明确拒绝（fail-closed），不得并发执行。
3. **同会话单写者**：同一会话的日志仍由每会话 flock 保证；两个进程写同一会话时，后者必须
   **显式失败并给出可观测信号**（不得静默、不得写坏），且不得影响该进程写**其它会话**。
4. **P0 行为不回归**：日志先行、`LogPersisted`、stale-epoch 预检/回滚、维护句柄零写入全部保持。
5. **可观测**：索引滞后（`index_deferred`）、BUSY 重试、会话写冲突均有结构化日志可 grep。

## 契约条目

| ID | Given | When | Then | 红相现状（基线必失败） |
|---|---|---|---|---|
| D1 | 进程 A 已持有库级维护锁（模拟 `rebuild-index` 进行中） | 进程 B 打开 store 并提交一轮普通写入 | B **不降级只读**，写入成功（WAL 多进程串行写） | 现基线：B 抢不到 `sessions.db.lock` ⇒ `read_only=true` ⇒ 整库只读 |
| D2 | 同上 | 进程 B 尝试 `rebuild_index` / migrate | B **拒绝**并返回明确错误（`maintenance_lock_busy`），不并发执行 | 现基线：靠同一把锁顺带实现，语义未区分「维护」与「日常写」 |
| D3 | 两个进程同时写**不同**会话 | 各自 append_turn | 两者都成功，互不阻塞 | 现基线：第二个进程整体只读 ⇒ 失败 |
| D4 | 两个进程同时写**同一**会话 | 后者 append_turn | 后者显式失败（`session_writer_busy` 可 grep），前者数据完整；后者可继续写其它会话 | 现基线：整体只读，失败但无区分、且无法继续别的会话 |
| D5 | 日常写路径 | 任意一次提交 | 不再调用 `acquire_single_instance_lock`（仅维护路径调用） | 现基线：`connection()` 首次即抢锁并据此降级 |

## 接口变更

- `SqliteSessionBackend`：
  - `is_read_only()` 语义收窄为**仅** `storage_read_only`（维护/诊断句柄）；
  - 新增 `maintenance_lock(db_path) -> Result<MaintenanceGuard, MaintenanceBusy>`（独占、非阻塞）；
  - 日常连接不再持有 `instance_lock`；`try_recover_instance_lock` 删除或仅供维护路径。
- 日志层：会话写冲突错误类型化为 `SessionLogError::Locked`（已存在）→ 上层映射为
  `SessionError::SessionWriterBusy`（新），日志行 `session_writer_busy: session=…`。

## 测试面（test-agent 独占）

新文件 `crates/pony-agent-core/tests/p1_multiprocess_writers.rs`，用例名固定：
`d1_writer_not_degraded_when_maintenance_lock_held`、`d2_maintenance_refuses_when_lock_held`、
`d3_two_processes_write_different_sessions`、`d4_same_session_second_writer_fails_explicitly`、
`d5_daily_write_path_does_not_take_instance_lock`。
跨进程用 `std::process::Command` 起测试辅助二进制（`tests/helpers` 或 `--bin`）或 `fork`；
**禁止**只做同进程假并发（同进程 flock 对同一 OFD 复用，测不出真实冲突）。

## 验收门禁

`cargo test --lib` + `--test p1_multiprocess_writers` + P0 的 `p0_log_first_durability` +
`session_log_*` / `pa174` / `session_row_heal`（判 pre-existing）/ `cargo check --all-targets` 全绿。

## 实现落点（Lead 核实版，基线 `e043f32f`）

| 改动 | 文件 | 基线锚点 |
|---|---|---|
| 日常连接不再抢库级锁：`connection()` 删掉 `acquire_single_instance_lock` 与 `read_only` 降级分支；统一 `journal_mode=WAL / synchronous=NORMAL / busy_timeout=5000 / wal_autocheckpoint=256` + `ensure_schema` | `agent/sqlite_session.rs` | `:299-320` |
| `is_read_only()` 语义收窄为 `storage_read_only`；删除 `read_only` 原子量与 `instance_lock` 字段 | `agent/sqlite_session.rs` | `:209-217`、`:2675` |
| 新增维护锁：`MaintenanceGuard` + `acquire_maintenance_lock(db_path)`（**沿用既有 `<db>.lock`**：不新增锁文件、与旧版本进程兼容；`try_lock` 非阻塞，抢不到返回 `MaintenanceBusy`，Drop 释放） | `agent/instance_lock.rs` | 复用 `:67` 的 flock 原语 |
| 维护入口加锁：`rebuild_index*` / migrate / backfill 非 dry-run 路径先取维护锁；`dry_run` 不需要（零写入） | `agent/session/fold.rs`、`bin/cli.rs` | `fold.rs:1745`、`cli.rs:1306/1423` |
| 删除 `try_recover_instance_lock` 及其两处调用 | `agent/sqlite_session_commit.rs`、`agent/sqlite_session.rs` | `:696`、`:2734`、`:2817` |
| `open_ask_store` 跳过条件改为 `storage_read_only()`（日常实例不再跳过） | `agent/session/store.rs` | `:224-233` |
| 会话写冲突类型化：日志 `Locked` → `SessionError::SessionWriterBusy`，日志行 `session_writer_busy: session=…` | `agent/session/store.rs`、`agent/session/log_store.rs` | `:641-646` 失败分支 |
| `index_deferred` / BUSY 重试计数结构化日志（可 grep 汇总） | `agent/sqlite_session.rs` | `:2755-2795` |

补充约束：
- `SqliteSessionBackend` 的 `storage_read_only` 语义不变（维护/诊断零写入）。
- 跨进程并发写索引由 SQLite WAL 串行化；**不得**再引入任何"进程级全局写锁"。
- 会话级单写者由 `SessionLogStore::open_write` 的 flock 保证（已存在，不改）。
- P0 契约（C1–C9 / R1–R6）全部保持不回归。

## 已知风险与处置（P1 引入的新并发面）

| 风险 | 说明 | 处置 |
|---|---|---|
| R-A 旧 JSON 迁移 TOCTOU | `migrate_from_json` 是「先 `SELECT COUNT(*)` 再导入」，两个进程同时首次启动可能重复执行迁移 | 迁移本身按主键覆盖、幂等；重复执行只多花时间。**要求**：迁移入口也取维护锁（与 rebuild 同级），或改为 `INSERT OR IGNORE` + 迁移完成标记 |
| R-B 并发首建表 | 多进程首次 `ensure_schema` | 已有 `busy_timeout=5000` + `CREATE TABLE IF NOT EXISTS`，后到者等待后成功；需在绿相日志里留一条双进程首启证据 |
| R-C WAL 检查点竞争 | 多进程写时 `wal_autocheckpoint` 并发 | SQLite 自身串行化 checkpoint；若出现 `SQLITE_BUSY` 由既有 3 次退避兜底 |
| R-D 会话写冲突可观测性 | 同会话第二写者 | 必须返回 `SessionError::SessionWriterBusy` 且日志含 `session_writer_busy`（D4 断言） |

## 契约修订（Lead，实现对齐）

- 维护锁文件名：**沿用 `<db>.lock`**（原 E5 单实例锁文件路径），不新增 `<db>.maintenance.lock`。
  理由：① 与已存在的锁文件/旧进程兼容；② 少一个文件少一处状态；③ 语义靠**谁去抢**区分
  （只有维护入口抢），不靠文件名区分。
- 测试侧因此同时兼容两种路径（`p1_helper_hold_maintenance_lock` 锁 `<db>.lock`）。
