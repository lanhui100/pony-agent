# 契约矩阵：ADR 0107 · P0 止血波次（wave/p0-0107-log-first）

基线：`c4a5bd81`（隔离 worktree `/home/dm/pony-agent-worktrees/wave-p0-0107`，不碰主工作区未提交改动）
真值来源：`docs/decisions/0107-session-scoped-writer-lease-and-log-first-durability.md` §5 P0

## 不变量（必须同时成立）

1. **日志先行**：会话 JSONL 是历史真源；索引（SQLite）不可写时，日志仍必须写。
2. **失败域隔离**：索引失败只允许产生「索引滞后」，不得让本轮历史从内存消失。
3. **日志失败仍 fail-loud**：日志写失败 ⇒ 整批失败 + 本轮不入内存（语义与今日一致）。
4. **可写实例零回归**：正常路径（日志+索引都成功）行为、返回值、落盘内容与今日完全一致。

## 契约条目

| ID | Given | When | Then | 红相现状（基线必失败） |
|---|---|---|---|---|
| C1 | 后端处于只读降级（`is_read_only()==true`，模拟抢不到库锁） | 提交一批含 `AppendMessage` 的命令 | 该会话 JSONL 出现对应行；SQLite 无新增行 | `agent/sqlite_session_commit.rs::append_session_log_to_file`（基线 :792-801）`is_read_only ⇒ return Ok(())` ⇒ 无日志 |
| C2 | 日志写入成功、索引持久化失败 | `SessionStore::append_turn_fallible` | 返回 `Ok`；内存 `session.history` 追加 user+assistant；记录「索引 deferred」告警 | 索引失败 ⇒ `return Err` 早于 live 提交 ⇒ 历史不入内存 |
| C3 | 日志写入失败 | `append_turn_fallible` | 返回 `Err`；内存不变 | 已满足（回归保护，防 C2 放宽过头） |
| C4 | 可写后端（日志+索引均成功） | `append_turn_fallible` | 返回 `Ok`；历史正确；索引行正确 | 已满足（零回归保护） |

## 接口变更（唯一新增面）

`PersistCommandOutcome` 增加 `LogPersisted`（日志已落盘、索引未应用）。语义：
- 仅当「日志成功 + 索引失败」时返回；
- 调用方（`append_turn_fallible` / `append_failed_turn_fallible`）必须：提交内存 live 状态 → 打结构化工单日志 → 返回 `Ok`（不调 `save_session_state_to_backend`）；
- 其余 outcome 语义不变；`Failed` 仍表示「日志或索引任一失败且日志未落地」。

## 测试面（test-agent 独占写域）

新文件：`crates/pony-agent-core/tests/p0_log_first_durability.rs`
- 用例名固定：`c1_read_only_backend_still_writes_session_log`、`c2_index_failure_still_advances_memory_history`、`c3_log_failure_keeps_memory_untouched`、`c4_writable_path_unchanged`
- 注入点：`SessionBackend` trait（测试侧实现包装/门面），不 mock 文件系统；C1 用真实 `SqliteSessionBackend` 只读句柄或等价的 `is_read_only` 后端。
- 断言必须是行为级（日志文件行内容 / 内存 history 长度与角色序列），禁止只断言"没报错"。

## 验收门禁

`cargo test --offline -p pony-agent-core --test p0_log_first_durability` 全绿 +
既有相关套件不回归：`session_log_wiring`、`session_read_from_log`、`session_log_store`、`session_event_carriers`、`pa174_history_wiring_and_followup_contract`。

## 实现落点（Lead 核实版，行号以 `c4a5bd81` 为准）

| 改动 | 文件 | 基线锚点 |
|---|---|---|
| 新增 outcome 变体 `LogPersisted`（日志已落盘、索引未应用） | `crates/pony-agent-core/src/agent/session/backend.rs` | `enum PersistCommandOutcome` :248-255 |
| 删除 `is_read_only` 短路，只读也写日志 | `crates/pony-agent-core/src/agent/sqlite_session_commit.rs` | `append_session_log_to_file` :792-801 |
| 日志成功后的索引失败映射为 `LogPersisted`（含 busy 重试耗尽） | `crates/pony-agent-core/src/agent/sqlite_session.rs` | `persist_commands_batch` :2728-2795（日志在 :2738 已先行写入） |
| 处理新 outcome：提交内存 live + 结构化 warn，跳过 `save_session_state_to_backend` | `crates/pony-agent-core/src/agent/session/store.rs` | `append_turn_fallible`（`:641` 判定、`:653` 内存提交）、`append_failed_turn_fallible`（约 `:719`） |

补充约束：
- `Failed` 仍表示「日志未落地（或日志写失败）」；不得把日志失败映射成 `LogPersisted`。
- `SessionBackend::persist_commands_batch` 的默认实现（File/Memory）不变。
- `store.rs` 在 `LogPersisted` 分支必须打一条含 `session_id`/`outcome` 的结构化日志（可 grep 断言）。

## P0.1 整改（L3 反审 FAIL 后追加，2026-11-09）

复核证据：`/tmp/diag/review-p0-0107.json`（FAIL：3 BLOCKING + 3 ADVISORY）。以下为整改契约，**优先于上文未覆盖处**：

| ID | 问题 | 修复契约 | 验收 |
|---|---|---|---|
| R1 | `apply_persist_command_tx` 的 stale-epoch 拒绝返回 `Err(String)` → 被 `index_failure_after_log` 吞成 `LogPersisted`（内存误前进） | 让 apply 层返回**类型化**结果：stale-epoch ⇒ `PersistCommandOutcome::StaleEpoch`；其余应用错误 ⇒ `Failed`。`index_failure_after_log` 只改判 `Failed`，`StaleEpoch` 必须透传 | `persist_command_writes_normalized_tables_and_checks_epoch` 绿（old epoch → `StaleEpoch`/非 Ok），且 `store.rs:645` 对 `StaleEpoch` 仍 `Err` 不入内存 |
| R2 | 门禁漏跑 `--lib`，2 条断言红 | ① 修复 R1 后 epoch 用例应自然转绿；② `flush_events_rolls_back_on_injected_failure` 的 outcome 断言按新语义从 `Failed` 改为 `LogPersisted`（并保留其"事务回滚无残留"断言），在 `.dev-team/green-phase-p0-0107.log` 显式登记「已声明的语义变更」 | `cargo test --lib` exit 0（1405 passed） |
| R3 | 降级实例 `LogWriter` 长期缓存并持 flock ⇒ 主可写实例写同一会话 `Locked → Failed → 整轮回滚`（破坏"可写实例零回归"） | 降级实例（`is_read_only() && !storage_read_only()`）改走**按批开/放** writer（append 后立即释放 flock）；可写实例保持缓存 writer 不变 | 新增用例：降级实例 append 一次后释放 flock（主实例随后同会话 append 成功）；现有并发/日志套件不回归 |
| R4 | `new_read_only`（`--dry-run`/诊断/测试夹具）在删除短路后会真写 `session.jsonl`，破坏 E8「零写入」 | 短路**只对 `storage_read_only` 保留**（维护句柄不写日志）；锁降级实例仍写日志 | `session_log_wiring.rs:196` 等维护句柄用例不出现日志写入；`--dry-run` 路径零写入 |
| R5 | `append_commands` 在 `grouped.is_empty()` 时 `Ok(())`（零字节）仍被判 `LogPersisted` | 日志层返回"是否实际写入"；零写入时不得判 `LogPersisted`（应 `Failed`） | 构造 `RemoveSession`-only 批：outcome 不得为 `LogPersisted` |
| R6 | `append_failed_turn_fallible` 未打 `index_deferred` 告警，executor 声称的 `persist_session_and_trace_change` 覆盖对 `ReplaceAll` 不可达 | 在 `append_failed_turn_fallible` 的索引失败路径补同一告警（不改落盘顺序/返回值） | grep 可得告警；行为零变更 |
