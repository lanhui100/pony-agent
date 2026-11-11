# Agent Note: 会话级写者租约与日志优先持久化（多窗口/多智能体并发，预留远程 agent）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0107-session-scoped-writer-lease-and-log-first-durability.md` 之归档快照，仅供溯源。

> 与既有决策的关系：**递进而非推翻 ADR 0099，取代 ADR 0106 的锁语义**（见 §5 P1 与「实施记录」）。ADR 0099 确立「每会话 append-only JSONL 是真源、SQLite 是可重建索引」；ADR 0106 确立「单写者冲突 fail-closed、提示真实、启动有预算」，消除「静默丢数据」。但 0106 把并发单元钉在**库级**，使「桌面端 + TUI + CLI + 多智能体」无法同时可写——本篇把并发单元从**库**降到**会话**，并保留 0106 的 fail-closed 防线（粒度变细）。

## 1. 背景

事故（2026-11-09，详见 `docs/analysis/turn-history-missing-rca-pa_01a1bfdf-20261109.md`）：
第二个 `pony` 实例抢不到 `sessions.db.lock` → 静默降级只读 → 只读实例**连 JSONL 日志也不写**
（`sqlite_session_commit.rs:803` `is_read_only ⇒ return Ok(())`）→ `append_turn` 两阶段暂存提交
在持久化失败处整轮回滚（`store.rs:641/643` 早于 `:653` 的 live 写入）→ 该轮 user/assistant
**既不落盘也不进内存** → 下一轮上下文为空（trace：`llm.request.messages` 恒 6、`agent.turn.sequence` 恒 0）。

需求侧：用户**经常需要多窗口、多会话、多智能体同时运行**；未来还要支持**远程 agent**
（远程 Linux 上跑 ponycli，桌面端作为客户端）。库级单写者与这两个需求都不兼容。

## 2. 目标 / 非目标

**目标**
1. 多窗口/多会话/多智能体可同时工作，历史零丢失。
2. 索引（SQLite）故障只影响**新鲜度/启动速度**，绝不影响**历史存在性**。
3. 持久化失败不得连带丢弃内存中的本轮历史。
4. 保留远程 agent 扩展空间：存储后端与写者租约可替换，真源归属 agent 所在处。

**非目标**
- 本篇不实现远程 agent（只预留接口与语义）。
- 不改变 JSONL 日志格式与 fold 重放语义（保持可重放、可对拍）。
- 不做本篇之外的库瘦身（1.2 GB / WAL 债已由 ADR 0106 登记独立维护窗口）。

## 3. 核心不变量（设计契约）

1. **真源唯一**：每会话 append-only JSONL 是历史唯一权威；SQLite 是可重建投影。
2. **写路径定序**：`日志 append+fsync 成功` → `内存 live 提交` → `索引更新（可重试/异步）`。
   索引失败**不得**回滚前两步。
3. **失败域隔离**：索引失败 ⇒ 标记 deferred + 重试 + 可观测；仅**日志**失败才 fail-loud。
4. **并发按会话**：写者租约（带 epoch fencing）按会话获取；不同会话/子代理互不阻塞。
5. **后端可替换**：真源与索引各自抽象为接口，本地 FS+SQLite 与远程 API 同构。

## 4. 候选方案 / Alternatives considered

**A. 维持库级单写者 + fail-closed（ADR 0106 现状）**
优点：已实现、简单、无并发写风险。
落选原因：直接拒绝多窗口/多智能体，不满足需求；远程场景更无法成立（跨机器 flock 无意义）。

**B（选中）会话级写者租约 + 日志优先 + 索引异步**
优点：并发单元与日志文件的物理边界一致（每会话一文件，天然单写者）；索引故障不再致命；
写者租约抽象可直接平移到远程服务端租约。
代价：需要把「库级锁」拆成「会话级租约 + 索引多进程串行写」，并给索引加 lag 观测。

**C. 直接上集中式存储服务（所有端连一个服务）**
优点：并发/远程天然解决。
落选原因：对当前单机多窗口需求是过度工程；且推翻 0099 的本地日志真源，迁移成本高。
B 的接口抽象保留了未来演进到 C 的路径，无需现在付出 C 的成本。

## 5. 决策（分层落地）

### P0 止血（不改并发模型，先堵住「静默丢历史」）
1. **只读降级仍写日志**：删除 `sqlite_session_commit.rs:803` 的 `is_read_only ⇒ return Ok(())`
   静默跳过；抢不到库锁 ⇒ 进入 **log-only 模式**：JSONL 照写（每会话单写者，安全）、索引标记 deferred。
2. **内存 live 提交与索引解耦**：`append_turn_fallible` 在日志成功后即 `sessions.insert`（内存前进），
   索引失败进重试队列；不再因索引失败整轮回滚（修掉 `store.rs:641/643` 的连带丢弃）。
3. ADR 0106 的启动预检/真实提示/启动预算**全部保留**，仅把拒绝条件从「库被占」改为「**目标会话**被占」。

### P1 会话级租约 + 索引多进程可写（解决多窗口/多智能体）
1. **会话级写者租约**：`session-logs/<ws>/<session>/writer.lock`（flock per session）+ epoch；
   持租约者独占该会话写；不同会话/子代理并行写互不阻塞。
2. **SQLite 索引多进程串行写**：WAL + `busy_timeout` + 短事务 + `SQLITE_BUSY` 重试；
   移除「库级锁抢不到 ⇒ 整库只读」。本地文件系统上 SQLite 本就支持多进程串行写。
3. **库级锁只保留给破坏性维护**（rebuild / migrate / VACUUM），独立 `maintenance.lock`。
4. **索引 lag 可观测**：`projection_lag` 指标 + UI 提示「索引落后，列表可能不是最新」。

### P2 远程扩展预留（只留接口与语义，不实现）
1. **存储后端抽象**：`SessionLogStore`（append / read / replay）+ `SessionIndexStore`
   （project / query）；本地实现 = FS+SQLite，远程实现 = HTTP/gRPC，同一 trait。
2. **写者租约抽象**：`Lease { holder, epoch, fence }`——本地 flock+epoch，远程服务端租约；同构。
3. **读取路径同构**：快照 + 事件流 + 断点续传，本地远程共用。
4. **真源归属原则**：agent 所在处持有真源；远程时桌面端只保留**可重建的只读缓存**。

## 6. 远程场景映射（证明扩展空间成立）

- 远程 ponycli 持有真源（session.jsonl + 本地索引）——P1 的会话级租约原样工作。
- 桌面端通过协议读快照/事件流、写「追加事件」请求——复用 P2 的 `SessionLogStore` 远程实现。
- 写者冲突用服务端租约 + epoch fencing——复用 P1 的 `Lease` 语义，仅换实现。
- 本地缓存损坏/丢失不影响数据——0099 的「索引可删可建」直接兑现。

## 7. 验收标准（机器可判）

1. 双开 TUI（同 HOME）：第二实例**历史正常增长**（`llm.request.messages` 逐轮 +2），无只读降级。
2. 索引只读/损坏：历史仍可写可读；`rebuild_index` 后 fold 与 live 逐字段一致。
3. 同一会话双开：后开者**仅该会话只读**且 UI 显式提示；其他会话不受影响。
4. 多智能体并行：各 `subagent-<id>` 会话独立写，无库级锁竞争日志。
5. 索引故障注入：本轮内存历史不丢，UI 有「未落库/待补写」告警而非静默。

## 8. 影响

- 锁语义变化：库级全局写锁 → 会话级租约 + 维护锁。旧实例只认库级 flock，需**同版本升级**，
  混跑期间旧实例仍按库级锁拒绝启动（向后兼容的保守行为，可接受）。
- 新增可观测：`projection_lag`、`log_only_mode`、`writer_lease_denied{session}`、索引重试计数。
- 代码落点：租约原语入 `agent/instance_lock.rs`（或新 `agent/session/lease.rs`）；
  日志优先与内存解耦入 `session/store.rs::append_turn_fallible`、`sqlite_session_commit.rs`；
  后端抽象入 `session/backend.rs`（`SessionBackend` 拆分）。
- 风险：多进程写 SQLite 需保证短事务、避免长 VACUUM 占锁；网络文件系统（NFS）上仍禁止 WAL 共享
  （远程场景走 API 而非共享文件，已规避）。

## 实施记录（2026-11-10 合入 main）

- **P0 已落地**：只读降级仍写会话日志；`LogPersisted`（日志已落盘、索引滞后）不再回滚内存历史；
  stale-epoch 类型化 + 写日志前 fail-closed 预检 + 事务拒收时持锁回滚本批字节。
- **P1 已落地**：日常写不再抢 `<db>.lock`（改由 WAL + `busy_timeout` 串行化）；`<db>.lock`
  收敛为维护锁（`acquire_maintenance_lock`）；同会话并发写返回 `SessionError::SessionWriterBusy`；
  旧 JSON 迁移改为单事务幂等（`BEGIN IMMEDIATE` + 完成标记）；`upsert_session` 加 `BEGIN IMMEDIATE`
  与 BUSY 有界重试；`index_deferred` 提供可机读计数。
- **取代 ADR 0106 的锁语义**：退役 `enforce_startup_permission` 启动预检（P1 后不再有「第二实例
  必须拒绝启动」），删除 `agent/instance_lock.rs`；保留 0106 的启动预算 / `startup_meta` /
  退出提示真实性等无关能力。
- **同时修复 0106 带入的启动预算缺陷**：预算 deadline 原为进程级 `OnceLock` 冻结 ⇒ 长驻进程
  后续装载永久跳过历史重建（＝「新一轮没有历史」）。改为每次 `load_store` 重起算。
- 验收：`--lib` 1421 passed；`p0` 7+2 ignored；`p1` 5+2 ignored；`pa174` 13 passed；
  `session_row_heal` 2 passed（长期 pre-existing 失败一并消失）；`tui_persistence_truthfulness` 95 passed。

