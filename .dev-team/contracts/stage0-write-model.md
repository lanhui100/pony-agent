# 契约：ADR 0099 阶段 0 · 写模型收口（stage0-write-model）· v1

> 状态：**协议冻结**（Protocol Agent / L2-P，2026-11-08）
> 真源：`docs/decisions/0099-session-storage-event-sourced-refactor.md` §「阶段 0 · 止血与收口」
> 本文件是阶段 0 的**唯一契约真源**：实现（Executor）与验收（Tester）以本文件为准，不以口头约定为准。
> NFR 基准：`.dev-team/nfr-baseline-stage0-write-model.json`（严格 schema + 校验器，见 §4）。
> 纯度边界：本阶段只冻结**契约 / 空桩 / NFR 基准**，不含任何业务逻辑实现。

## 0. 交付边界（谁冻结了什么）

| 类别 | 内容 | 状态 |
| :--- | :--- | :--- |
| 已落码（Protocol） | `CommitOutcome`、`BackfillReport`、`SessionBackend::commit_boundary` 默认实现、`SessionVersion`、`SessionVersionMap`、公开 re-export | ✅ 编译通过（`cargo check -p pony-agent-core` exit 0） |
| 仅契约未落码（阶段 0 Executor） | `PersistedStore.versions` 字段、`SqliteSessionBackend::commit_boundary` 覆写、单实例文件锁、`is_busy_error`、`backfill_orphan_sessions`、`load_normalized_session` trace 源切换、调用点收敛 | ⛔ 尚未落地（anchor 见 §5） |

**取证事实（阶段 0 的因果前提，全部为现状实测）**：

| 事实 | 落点 |
| :--- | :--- |
| 每 turn 构造整库 `PersistedStore` 后整包写盘 | `session/store.rs:2409`（`save_to_backend`）、`:2462`（`save_session_state_to_backend`）；`PersistedStore` 构造在 `:2428` / `:2489` |
| 生产调用点实测 `save_to_backend()` **18 处** + `save_session_state_to_backend(…)` **5 处** = 23（测试文件另有 2 处） | 均为 `session/store.rs`（ADR 正文记 19，口径为「每 turn 全量提交点」，与本文件口径不同，不是缺陷） |
| `write_full_store` 已移除「删除快照外会话」（ADR 0094），但**无版本守门** | `sqlite_session.rs:599`，删除注释在 `:653` |
| `contains("busy")` 重试是死分支（rusqlite 实际报 `database is locked`） | `sqlite_session.rs:2661`、`:2687` |
| `load_normalized_session` 的 trace 取自 legacy `session_turn_traces` | `sqlite_session.rs:2901`（`load_normalized_session`）→ `:1960`（`read_all_session_traces`，定义 `:1044`） |
| 无单实例：桌面端与 TUI 可同写 `~/.local/share/PonyAgent/sessions.db` | 会话库落点 0 处锁；全库唯一 `flock` 在 goal store（`agent/goal.rs:176-192`），与 `sessions.db` 无关 |

## 1. 冻结 API（已落码，可被外部测试直接命名）

> ⚠️ `session/mod.rs` 里 `mod backend;` / `mod types;` 是**私有模块**，因此公开路径是**短路径**（下表），
> 不是 `session::backend::CommitOutcome`。「私有模块 ⇒ 必须 re-export 才可达」是本次唯一越出
> backend.rs/types.rs 的改动（2 行 `pub use`，纯 additive）。

| 符号 | 公开路径 | 形态 | 落点 |
| :--- | :--- | :--- | :--- |
| `CommitOutcome` | `pony_agent_core::agent::session::CommitOutcome` | `Committed` / `StaleVersion { expected: u64, actual: u64 }` / `Failed` / `Unsupported`；derive `Clone, Copy, Debug, PartialEq, Eq` | `session/backend.rs:197` |
| `BackfillReport` | `pony_agent_core::agent::session::BackfillReport` | `{ backfilled: Vec<String>, retired: Vec<String>, checksum_mismatch: Vec<String> }`；derive `Clone, Debug, Default, PartialEq, Eq` | `session/backend.rs:214` |
| `SessionBackend::commit_boundary` | trait 方法（`SessionBackend` 已公开） | `fn commit_boundary(&self, store: &PersistedStore) -> CommitOutcome`；**默认实现返回 `Unsupported`** | `session/backend.rs:268` |
| `SessionVersion` | `pony_agent_core::agent::session::SessionVersion` | `pub struct SessionVersion(pub u64)`；`#[serde(transparent)]`；derive `Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize` | `session/types.rs:41` |
| `SessionVersionMap` | `pony_agent_core::agent::session::SessionVersionMap` | `pub type SessionVersionMap = HashMap<String, SessionVersion>` | `session/types.rs:48` |

`commit_boundary` 的**最终目标语义**（阶段 0 Executor 覆写后）：

| 返回值 | 触发条件 | 写入行为 |
| :--- | :--- | :--- |
| `Committed` | 快照内所有「未墓碑 / 未被显式删除」会话的期望版本 == 库内版本 | 各会话按 CAS 写一次并 `version + 1`；墓碑/已删会话**跳过、永不 INSERT** |
| `StaleVersion { expected, actual }` | 任一受限会话期望版本 ≠ 库内版本 | **整包零写入**（不覆盖新 turn、不删除、不复活） |
| `Failed` | I/O / 序列化 / 事务失败 | 事务回滚；调用方必须可观测上报，禁止 `let _ =` |
| `Unsupported` | File / Memory 后端（默认实现） | 调用方回退 `save_store`（保持既有语义与可编译性） |

`load_store()` 之后**直接** `commit_boundary(&fresh)` == `Committed`（无需先写过一次）。

## 2. 裁决：版本载体 = `PersistedStore` 侧表，**不是** `SessionState::version`

**决定**：版本以侧表 `PersistedStore.versions: SessionVersionMap` 承载（`#[serde(default)]`，阶段 0 Executor 落码，`session/store.rs:120` 的 `PersistedStore` 定义处）。

理由（按权重排序）：

1. **构造点爆炸面**：`SessionState` 全库有 **24 处**结构体字面量构造点（`session/store.rs` / `sqlite_session.rs` / `session/tests.rs`，0 处使用 `..Default::default()`），加字段必须同步改 3 个文件、含 2 个不得由本阶段触碰的测试文件；`PersistedStore` 只有 **1 处定义 + 4 处显式构造**（`store.rs:2428` / `:2489`、`sqlite_session.rs:2393` / `:2795`，另有 1 处测试用 `..Default::default()` 免改）。
2. **序列化面污染**：`SessionState` 是 `sessions` 表 blob 的载荷，版本号是**存储层元数据**，写进 blob 会让「会话内容」与「存储版本」绑定，阶段 2 真源切换时无法独立演进（blob 将降级为导出产物，版本此时应消失）。
3. **语义归属**：CAS 是**单写者提交边界**的属性，天然属于 `SessionBackend`/`PersistedStore` 一侧；放进 `SessionState` 会让内存态与持久态版本互为镜像，产生两份真值。
4. **最小闭包**：侧表把阶段 0 的改动限制在 5 个生产构造点 + 1 个定义点，符合「最小闭包」底线。

**同时否决**新增 `save_store_if_version(&self, store, expected) -> Result<(), SessionWriteError>`：`save_store(&PersistedStore) -> Result<(), String>` 是既有公开契约（多后端 + 多处调用），改签名会把「版本守门」扩散成全量改调用点；`commit_boundary` 作为**单一新入口**吸收版本语义，旧 `save_store` 保持不动（File/Memory 走默认 `Unsupported` 外无行为变化）。

## 3. 五条不变量（I1–I5）

| ID | 不变量 | 判定语义 | 机器验收 | 违反后果 |
| :--- | :--- | :--- | :--- | :--- |
| **I1** | 每 turn 全库提交 **≤1 次** | 一个 turn 的全部状态变更（消息/trace/事件/节点）在**一个**提交边界内落盘；19/23 个 `save_to_backend` / `save_session_state_to_backend` 调用点收敛到该边界，禁止 turn 内多次整包写 | CountingBackend 断言：一次 `append_turn` 后 `save_store` 调用计数 == 1（`commit_boundary` 返回 `Unsupported` 时回退 `save_store`） | 写放大 790×（102 MB/turn）、跨会话事务互相拖住 |
| **I2** | `version` 不匹配 ⇒ 返回 `StaleVersion` 且**不得写入** | 提交前对每个受限会话做 `UPDATE … WHERE version = ?expected`；`changes() = 0` ⇒ 整包零写入并返回 `StaleVersion { expected, actual }` | 双段断言：fresh `load_store()` → commit ⇒ `Committed`；同一旧 store 再 commit ⇒ `StaleVersion`（防「永远 Committed」与「永远 StaleVersion」两种假绿） | 旧快照覆盖新 turn（跨进程 LWW） |
| **I3** | 显式删除后任何旧版本快照**不得复活**该会话 | 提交时对「库内已墓碑 / 已显式删除」的会话**跳过、永不 INSERT**；删除只走 `remove_session`（legacy 行 + trace + 规范化行同事务） | 删除后喂入删除前快照 ⇒ 该 `session_id` 在 `sessions` / `normalized_sessions` 中仍不存在 | 会话「诈尸」：退出提示的 `pony resume <id>` 时有时无 |
| **I4** | 孤儿回填源只允许 `turn_events ∪ normalized_*` | 回填判据 = 规范化行（`normalized_messages` 条数 / 校验和）与事件流一致；**禁止**以 legacy `sessions` blob 为源（blob 是待废弃的快照，用它回填等于把残缺固化为权威） | 回填后：`normalized_sessions` 与 `sessions` 双向集合差 == 0；逐会话校验和一致；无法回填者进 `BackfillReport.retired` 并在 UI 明示 | 50 个孤儿会话（41%）永久不可见，或把半残数据升格为权威 |
| **I5** | busy/locked 必须按 `rusqlite::ErrorCode::DatabaseBusy / DatabaseLocked` 判定并退避 | `matches!(err, rusqlite::Error::SqliteFailure(e, _) if matches!(e.code, DatabaseBusy | DatabaseLocked))`；退避 50→100→200 ms，最多 3 次，超限 fail loud | `is_busy_error(&rusqlite::Error)` 单测（构造 `SqliteFailure`）；**禁止** `to_string().contains("busy")`（对 `database is locked` 永不匹配） | 现有死分支（`sqlite_session.rs:2661/:2687`）：锁冲突零重试、直接失败 |

> I5 的 `ErrorCode` 指 **rusqlite / libsqlite3-sys 的 `ErrorCode`**（已核 0.28/0.30 均含 `DatabaseBusy`、`DatabaseLocked`），
> **不是**本项目 `agent::error_code::ErrorCode`（该枚举无此二变体，且是 wire 错误码，与本约束无关；不得为对齐本约束去改它）。

## 4. NFR 基准

文件：`.dev-team/nfr-baseline-stage0-write-model.json`（实例）+ `.dev-team/nfr-baseline-stage0-write-model.schema.json`（严格 schema）
校验器：`.dev-team/nfr-lint-stage0.py`（schema + **未知字段拒绝** + 硬约束，退出码 0/1/2）

| 维度 | 基准值 | 说明 |
| :--- | :--- | :--- |
| `external_call_timeout_ms` | **3000** | 跨进程写等待总预算（文件锁获取 + busy 退避累计 + `busy_timeout`）；本地单次 SQLite 写不算外部调用 |
| `retry` | `max_attempts=3`、`backoff 50→100→200 ms`、`backoff_cap_ms=200` | 仅对 `DatabaseBusy` / `DatabaseLocked`；`StaleVersion` / `Failed` **不可重试**；禁止 `contains("busy")` |
| `concurrency_lock` | 单实例**文件锁**（`sessions.db.lock`，`flock(LOCK_EX|LOCK_NB)` / `LockFileEx`）+ session 级 **CAS**（`WHERE version = ?`） | 锁获取非阻塞：抢不到即降级只读并结构化日志，禁止忙等/抢占；`lock_acquisition_timeout_ms=2000 < watchdog=5000`；禁止在写事务内做 fold / `load_store` / 全量 trace 读 |
| `logging` | `format=json` + `trace_id`（字段名 `trace_id`） | 必落事件：`commit_boundary_committed` / `commit_boundary_stale_version` / `commit_boundary_failed` / `busy_retry` / `single_instance_lock_denied` / `orphan_backfill_summary`；`compliance_claimed=false`（沿用 PA-119 债务口径，不主张已满足） |
| `resource_release` | 见 JSON 6 条 | 写事务 ≤50 ms 且不含 fold/全量读；锁 guard 在所有 early-return 路径 drop；busy 重试有界且失败不得静默；`retired` 只标记不删行 |

**与全局基准的关系**：`.dev-team/nfr-baseline.json`（全仓累积基准，root `additionalProperties=false`）**保持不动**。阶段 0 的 `retry.max_attempts=3` / `concurrency_lock` 与全局既有条目（`max_attempts=1`、`concurrency_lock="mutex"`）冲突，改写全局会破坏既有 wave 声明与 `.dev-team/nfr-lint.py` 门禁，故独立成文件、独立严格 schema。

## 5. 阶段 0 Executor 残留落码点（anchor 精确）

| ID | 落点 | 交付形态 | 验收 |
| :--- | :--- | :--- | :--- |
| E1 | `session/store.rs:120`（`PersistedStore` 定义） | `#[serde(default)] pub(crate) versions: SessionVersionMap`；同步 4 处显式构造点（`store.rs:2428` / `:2489`、`sqlite_session.rs:2393` / `:2795`；`:3925` 走 `..Default::default()` 免改）与 `load_store` / `load_store_normalized` 的填充 | `cargo check` 通过；`load_store()` 后 `versions` 与 `sessions` 表版本列一致 |
| E2 | `sqlite_session.rs:2583`（`impl SessionBackend for SqliteSessionBackend`） | 覆写 `commit_boundary`：单事务 + 逐会话 CAS + 墓碑跳过 + 版本自增 | I1/I2/I3 断言；`Committed` / `StaleVersion` 双段 |
| E3 | `sessions` 表 DDL + `write_full_store`（`sqlite_session.rs:599`） | 新增 `version INTEGER NOT NULL DEFAULT 0` 列（`ALTER TABLE` 幂等迁移）；`UPDATE … WHERE conversation_id=? AND version=?` | 旧库幂等升级；重复迁移 0 报错 |
| E4 | `session/store.rs:2409` / `:2462` 及 23 个调用点 | 收敛为每 turn 一次 `commit_boundary`；其余调用点改为同边界内合并（或改走 `persist_commands_batch`） | I1：CountingBackend `save_store` 计数 == 1 |
| E5 | 新进程启动路径（`SqliteSessionBackend::new` 一侧） | `sessions.db.lock` 独占锁；抢不到 ⇒ 只读 + `single_instance_lock_denied` | 双进程并发：第二实例只读，库文件零写入 |
| E6 | `sqlite_session.rs:2661` / `:2687` | `pub fn is_busy_error(error: &rusqlite::Error) -> bool`（`pub mod sqlite_session` 内），两处死分支改调用它 + 50→200 ms 退避 3 次 | I5 单测；`contains("busy")` 全库 0 处 |
| E7 | `sqlite_session.rs:2901`（`load_normalized_session`），trace 源 `:1960` | trace 改读 `normalized_turn_traces`（表 DDL `:370`，写入 `:1266`）；legacy `read_all_session_traces` 不再参与 resume 路径 | `load_normalized_session(id).turn_trace_history.len()` == `normalized_turn_traces` 行数（legacy 表为空也能恢复） |
| E8 | 新入口（`SqliteSessionBackend`） | `pub fn backfill_orphan_sessions(&self) -> Result<BackfillReport, String>`；源 = `turn_events ∪ normalized_*`；无法回填 ⇒ `retired` | I4：双向集合差 == 0；`checksum_mismatch` 进报告不覆盖 |

**已落码但需 Executor 保持不变的契约**：`CommitOutcome` / `BackfillReport` / `SessionVersion` / `SessionVersionMap` 的形态与公开路径（本文件 §1）不得改；`save_store(&PersistedStore) -> Result<(), String>` 签名不动。

## 6. 非目标（阶段 0 明确不做）

1. **不改真源**：仍为 `sessions` blob + `normalized_*` 双写；`session_events` 升格、`writer_id/writer_seq`、`session_writers` 租约与 `fence_token` 属阶段 1/2。
2. **不做会话级分片锁**：SQLite 每库单写者，分片锁无法并行提交；ADR 0099 已裁决阶段 0 不动锁结构，只收敛提交边界、缩短事务。
3. **不删任何存量数据**：孤儿无法回填时只标 `retired`；物理清理走既有 `turn_events_archive` + retention。
4. **不重编码历史**：只保证旧库可读、可导出、可回落。
5. **不改 `agent::error_code::ErrorCode`**：本阶段不新增 wire 错误码（`DatabaseBusy/Locked` 是 rusqlite 侧概念）。

## 7. 物理收据（本契约冻结时实测）

| 门禁 | 命令 | 结果 |
| :--- | :--- | :--- |
| 编译 | `cargo check -p pony-agent-core` | **exit 0**（`Finished dev profile`，0 warning） |
| NFR 严格校验 | `python3 .dev-team/nfr-lint-stage0.py` | **exit 0** `[PASS]` |
| NFR 反例（非恒真） | 同上 `--instance` 指向 4 个注入反例（未知字段 / timeout=4000 / `on_mismatch` 去 CAS 语义 / `logging.format=text`） | **exit 1 ×4**（拒绝生效） |
| dev-team NFR lint | `dev-nfr-lint.py .dev-team/nfr-baseline-stage0-write-model.json` | **exit 0** `[PASS]` |
| dev-team NFR lint（全局，未改） | `dev-nfr-lint.py .dev-team/nfr-baseline.json` | **exit 0** `[PASS]` |
| 空桩纯度 | `dev-stub-lint.py backend.rs types.rs` | **exit 0**（该启发式只解析 .py/.ts/.js，Rust 属「检查数 0」口径；Rust 侧纯度人工核：新增函数体各 1 条语句，模块级仅类型定义，无默认实参表达式/装饰器/副作用） |
| 全局基准回归 | `python3 .dev-team/nfr-lint.py` | **exit 1（既有缺陷，非本改动引入）**：`$.metrics_snapshot` 未在 `.dev-team/nfr-baseline.schema.json` 声明（wave-metrics 提交遗留漂移）；本改动对该文件 **0 diff** |

## Alternatives considered

1. **`SessionState::version` 字段**（本契约 §2 的另一方案）：语义直观、与 blob 同行，但需同步 24 处结构体字面量（含 2 个测试文件）并把存储版本写进会话 payload，违反最小闭包且污染阶段 2 的 blob 降级路径 ⇒ 否决，改用 `PersistedStore.versions` 侧表。
2. **新增 `save_store_if_version(store, expected) -> Result<(), SessionWriteError>`**：版本语义显式，但改既有公开签名会扩散到所有后端与调用点，且与「单一提交边界」重复 ⇒ 否决，收敛进 `commit_boundary`。
3. **给 `agent::error_code::ErrorCode` 加 `DatabaseBusy` / `DatabaseLocked`**：看似统一，但该枚举是 wire 错误码注册表，SQLite 锁冲突不跨越线缆；且 rusqlite 已有同名 `ErrorCode`，双定义必然分叉 ⇒ 否决。
4. **直接覆盖 `.dev-team/nfr-baseline.json`**：符合「一个基准文件」直觉，但该文件是全仓累积基准且 root `additionalProperties=false`，阶段 0 的 `retry.max_attempts=3`、`concurrency_lock` 与既有声明冲突，覆盖会删掉既有 wave 断言并使 `.dev-team/nfr-lint.py` 变红 ⇒ 否决，改为阶段独立文件 + 独立严格 schema。
5. **阶段 0 就建墓碑表（`session_deleted` 事件）**：删除传播最彻底，但属阶段 1（D3）；阶段 0 用「跳过墓碑/已删除会话、永不 INSERT」的最小守门即可消灭「复活」 ⇒ 延后。
6. **为测试便利给 `PersistedStore` 加 `pub` 构造器 / 版本注入 seam**：会扩大阶段 0 公开面并诱导测试依赖内部结构；改为测试侧用 `load_store()` 真实快照（Tester 已按此口径编写） ⇒ 否决。

## 决策记录待办（越出本写域，交 Lead 处置）

- `.agents/notes/` 决策记录（常载命约第 1 条）需由 Lead 或授权者落地；本文件 §「Alternatives considered」为其素材。Protocol Agent 写域不含 `.agents/**`。
- `.dev-team/nfr-baseline.schema.json` 缺 `metrics_snapshot` 声明的既有漂移（§7 末行），属 wave-metrics 遗留，不在本阶段范围内，未修。
