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
