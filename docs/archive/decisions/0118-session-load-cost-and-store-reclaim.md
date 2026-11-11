# ADR 0118：会话库装载成本收口（折叠字节预检 / trace 单源兜底 / 维护预算按阶段起算）与库体回收（auto_vacuum=FULL）

Status: implemented

日期：2026-11-10
实施说明：[`.agents/notes/implemented/architecture/2026-11-10-session-load-cost-and-store-reclaim.md`](../../.agents/notes/implemented/architecture/2026-11-10-session-load-cost-and-store-reclaim.md)。
关联：[0115](0115-tui-first-frame-before-control-plane.md)（首帧先画；本 ADR 处理它留下的「装载本身仍 ~4.3s」，首帧口径不得回退）/ [0099](0099-session-storage-event-sourced-refactor.md)（D1 日志真源、E7 trace 主源、阶段 2 门禁「装载 ≤800ms」）/ [0107](0107-session-scoped-writer-lease-and-log-first-durability.md)（每次装载重起算自愈预算；维护锁只给破坏性维护）、[0106](superseded/0106-session-store-single-writer-fail-closed-and-startup-budget.md)（启动预算能力来源）。

## 背景

ADR 0115 把首帧降到 50~100ms 之后，**装载本身**仍是 2.9~4.3s（真实库 1.2GB / 154 会话 / 89k 事件、
session-logs 682MB）。上一轮把剩余成本登记为 `B047`（装载）与 `B046`（库体积），本轮按实测逐项收口。

### 装载侧：三笔可归因成本（同一沙箱消融 + 直接 SQL 计时）

| # | 位置 | 现象（真实库） | 代价 |
|---|---|---|---|
| F1 | `fold::fold_read_view` | 折叠按**时**预算（800ms）跑，超时语义是**整轮丢弃**已折叠结果。日志 682MB / 154 会话 ⇒ `session_fold_slow: elapsed=880~1050ms folded=10~13 skipped=141~144`，折出来的东西**一行没用上**，随后 `fill_index_only_rows_from_index_projection` 又把 153 个会话从索引投影全量读一遍 | ~0.9s + 全部日志 IO |
| F2 | `commit::read_traces_normalized` | 兜底步骤把**整张** `session_turn_traces` 读成 HashMap（397 行 / 222MB / 645ms），调用方再 `entry().or_insert(..)` 逐会话丢弃。该表在真实库上与 `normalized_turn_traces` 完全重叠（`legacy-only sessions = 0`）⇒ **全部白读** | ~0.65s / 245MB |
| F3 | `SqliteSessionBackend::load_store` | 维护预算 deadline 在**索引回填之前**取值，回填（1.5s 级、不受预算约束）把它提前吃光 ⇒ `attach`/`rebuild` **在入口即判过期**：每轮启动都落 `history_rebuild reason=budget_exhausted sessions=28`，既没修任何会话，也永远修不上（28 个会话重启后历史恒为空） | 0（死代码）→ 修复后 +0.57s，但真修 24 个会话 |

### 库体侧：`B046` 的两条断言（先立反证，避免走错方向）

- **空洞不拖慢启动**（已在 `B046` 记录）：同一快照 A/B（1.2GB 原样 vs `VACUUM INTO` 后 730MB），
  pty 内各 3 轮，首帧前累计读盘字节**完全一致**（3083MB）。SQLite 扫表只走 B-tree 页、不访问 freelist。
  故本条目是**磁盘占用**项，不是性能项。
- **空洞来源 = 删除 + `auto_vacuum=OFF`**：`DELETE` 一行 3.89MB `session_turn_traces` ⇒ `freelist_count`
  +1066 页，`page_count` 不变（文件不缩），随后插入小行**复用 0 页**。真实库 `freelist_count=114048`
  页（445MB / 39%），WAL 一度积到 431MB。

## 候选方案

### F1（折叠成本）

- **A（采纳）进入折叠循环前先做**字节**预检**：日志总量 > `DEFAULT_READ_FOLD_LOG_BYTES`（32MiB）⇒
  一次日志 IO 都不做，直接整轮降级为读索引视图。判定是**确定性**的（同一份库两次装载同结论，
  不随机器快慢跳变），且保留了「小库照常走日志真源」（新用户/小库日志通常 << 32MiB）。
- **B（落选）保留部分折叠结果（去掉 `outcome.folded.clear()`）**：能同时省时并让前 N 个会话走日志真源，
  但改变既有降级契约（「超预算 ⇒ 整轮按索引」），且与 `read_path_merge_with_zero_budget_degrades_to_the_index_view`
  用例的语义相反；更关键的是**解决不了吞吐问题**——实测折叠吞吐 ≈100MB/s（10 会话 / 88MB / 880ms），
  682MB 全折需 ~6.8s，而索引投影全量读只要 1.5s。让「更贵的真源」与「更便宜的完整投影」竞争启动窗口，
  收益方向本身是错的。
- **C（落选）把折叠挪到首帧之后的后台刷新**：架构上更彻底（日志真源不会因预算被排除），但要引入
  「装载完成后列表再变一次」的视图失效协议，属独立决策；本轮先把「注定被丢弃的 IO」停掉。

### F2（trace 兜底）

- **A（采纳）两步走**：先读规范化侧（主源，E7），再用 `SELECT DISTINCT session_id FROM session_turn_traces`
  （只走覆盖索引，0ms）拿到 legacy 侧出现过的会话，**只对规范化侧给不出 trace 的会话**逐会话回读。
  语义与旧的「整表读 + `or_insert`」逐字节等价（`or_insert` 本就只在规范化侧缺该会话时生效），
  真实库上回读集合为空 ⇒ 省掉全部 222MB。
- **B（落选）`WHERE NOT EXISTS (SELECT 1 FROM normalized_turn_traces ...)` 一条 SQL 搞定**：实测同样极快
  （245MB/646ms → 0.1MB/1ms），但 `NOT EXISTS` 只能判「该会话有无规范化行」，判不了「规范化行是否
  可用」（`raw_json IS NULL` 的占位行要先读大字段才能判，反而把 222MB 读回来）；而且语义会**静默变窄**
  （占位行会话不再兜底）。两步走能精确复刻「读得出才叫有」。

### F3（维护预算）

- **A（采纳）预算按阶段起算**：`load_store` 在索引回填**之后**调 `startup_repair::reset_startup_budget()`
  再进维护项（`run_budgeted_startup_self_heal`）。语义仍是「维护项有 1200ms 上限」，只是不再被不受
  预算约束的回填提前吃光。实测结果：`history_rebuilt_from_turn_events: 24`（原先 24 个会话历史恒空），
  代价 +0.57s（24 会话 ≈ 24ms/会话）。
- **B（落选）持久化游标（`B047` 原始候选③）**：游标只改变「这轮修哪批」，不减少工作量；而修复结果
  按既有契约是**内存态**（`startup_repair_deferred: 交给首次真实写入落盘`），未落盘的会话下一轮仍是空壳，
  游标只会让它们轮流「看起来修过」。真正能收敛的是持久化修复结果，那要动 index-only 轻量行契约，另立决策。
- **C（落选）保持现状（预算恒过期）**：等于承认 24 个会话永远没有历史——不是「下轮再补」，是永不补。

### B046（库体回收）

- **A（采纳）显式维护入口 `ponycli sessions vacuum`：`auto_vacuum=FULL` + `VACUUM`**，收尾
  `wal_checkpoint(TRUNCATE)`。实测（同机 SQLite 3.45.1，200MB 库删一行 4MB trace）：OFF 46ms/200MB/978 空闲页；
  INCREMENTAL 36ms/201MB/978 页（`incremental_vacuum` 只收 1 页，**基本收不动**）；FULL 50ms/**197MB**/**0 页**。
  即 FULL 用 +4ms 换「删除自己回收、永不累积」，而且**不再需要**在 `remove_session` 里追加任何回收调用。
- **B（落选）`auto_vacuum=INCREMENTAL` + 删除后 `PRAGMA incremental_vacuum(N)`**（`B046` 原始候选①②）：
  实测整表 DELETE 后 1956 页空闲，一次 `incremental_vacuum` 只减 1 页（多次调用仍 1 页/次）。
  承诺「空洞不再累积」无法兑现，且要多一条写路径调用。
- **C（落选）启动时自动迁移**：VACUUM 要独占 + 约 2 倍临时空间，挂在正常启动路径上等于把「打开应用」
  变成「可能失败的破坏性维护」，与 ADR 0107「维护锁只给破坏性维护」的口径冲突。
- **D（落选）只 VACUUM 不改 `auto_vacuum`**：省下当下 445MB，但空洞会再涨回来（等于白做）。

## 决策

1. `crates/pony-agent-core/src/agent/session/fold.rs`：新增 `DEFAULT_READ_FOLD_LOG_BYTES = 32MiB` 与
   `total_log_bytes()`（`stat` 级，不读内容）；`fold_read_view` 在折叠循环前预检，超预算 ⇒ 整轮降级并落
   `session_fold_precheck` 日志。降级语义（整轮按索引视图）**不变**，只去掉「读满 800ms 再丢弃」。
2. `crates/pony-agent-core/src/agent/sqlite_session.rs` + `sqlite_session_commit.rs`：`read_traces_normalized`
   的 legacy 兜底改为「按缺失会话逐会话回读」；删除 `read_all_session_traces`（唯一消费者就是它），
   新增 `legacy_trace_session_ids()`（自由函数，放 commit 模块，避免把行数堆进 `sqlite_session.rs` 的棘轮基线）。
3. 自愈编排拆为两段：`run_budgeted_startup_self_heal`（维护项，带 deadline）+ `fill_index_only_rows_from_index_projection`
   （不受预算约束）。`load_store` 在回填之后重置预算再进维护项，保证预算「按阶段起算」。
4. 新增维护入口 `ponycli sessions vacuum [--dry-run] [--db <path>]`：`--dry-run` 只读探针
   （`inspect_session_store`），apply 走维护锁 → `auto_vacuum=FULL` + `VACUUM` → `wal_checkpoint(TRUNCATE)`。
   退出码 `0/1/2/3`（3 = 维护锁被占）。
5. 写路径 checkpoint 由「恒 `PASSIVE`」改为「小 WAL 走 `PASSIVE`，WAL ≥64MiB 才尝试 `TRUNCATE`，且把
   busy 等待压到 50ms、抢不到立即退回 `PASSIVE`」——回收失败**不得**变成保存失败。
6. `remove_session` **不加**任何回收调用（FULL 模式已自带），保持写路径零改动。
7. **维护子命令的 `--help` 护栏**（本轮事故现场，见下）：`run_maintenance_subcommand_if_requested()`
   在任何 `--help` / `-h` 出现时一律让路；`ponycli sessions …` / `ponycli config …` 命名空间下打印完整用法
   并 `exit 0`。新增 `is_help_request()` 纯函数 + 2 例单测钉住。

> 事故记录（必须留档）：验证 `ponycli sessions vacuum --help` 时，由于维护子命令在常规参数解析**之前**
> 拦截且当时没有 help 判定，命令**真的执行**了破坏性维护，把真实库
> `~/.local/share/PonyAgent/sessions.db` 从 1144MB VACUUM 到 743MB（`integrity_check=ok`、
> 会话/事件/trace 行数均未变，`-wal` 归零）。数据无损失，但「读帮助」不该产生写操作——护栏即由此而来。
> 该缺陷对 `rebuild-index` / `migrate-logs` 等**所有**维护子命令同源生效，本次一并收口。

## 后果

机器回执（release 二进制、pty + tmux、同沙箱三轮；`splash` = 首帧，`ready` = 「正在装载会话库…」消失）：

- **首帧不回退**（ADR 0115 口径）：`ab-logs` 49/50/49ms、`ab-base` 52/55/52ms、`pv/A` 53/88ms。
- **装载完成时间**（真实日志场景 `ab-logs`）：7805（冷）/2970/2897ms；旧实现同沙箱 ≈4.0s
  （= 本次 2.93s − 修复后多做的 0.5s 维护 + 折叠 0.9s + legacy 兜底 0.65s 的还原估值，
  与上一轮 `fold elapsed=880~1050ms`、`rchar 645ms` 的直接计量一致）。
- **无日志场景**（`ab-base`）：默认 3784/3725/3510ms；`PONY_STARTUP_SELF_HEAL_BUDGET_MS=0` 时
  3250/3055/3007ms ⇒ 维护项成本 ≈0.57s，收益是 `history_rebuilt_from_turn_events: 24`（原先恒空）。
  净时间近乎持平（−0.65s 兜底 +0.57s 修复），但**内容不再丢失**——这是本 ADR 认定的正确取舍：
  「快速读错」不算提速。
- **库体**（真实库副本 1144MB / freelist 114048 页）：`ponycli sessions vacuum` → **697MB / freelist 0 /
  auto_vacuum 0→1**，耗时 5056ms；`PRAGMA integrity_check` = ok，行数不变（154 会话 / 89464 事件 /
  398+397 trace）。迁移后插入 4MB 行再删除：731.5MB → 735.5MB → **731.5MB**（FULL 自动回收，无需人工步骤）。
- **测试**：`cargo test -p pony-agent-core --bin pony` 119 passed / 0 failed；
  `--bin ponycli` 护栏 2 例通过；`--lib store_reclaim` 6 passed（含 WAL 阈值与 busy 回退）；
  `--test storage_stage0_recover_trace` 2 passed（新增「legacy 独有 trace 仍被兜底」反向守卫）；
  `--test session_read_from_log` 8 passed（含 `fifty_logged_sessions_load_within_budget_and_degrade_keeps_the_index_complete`）；
  `p0_log_first_durability` 7 passed、`storage_stage0_write_amplification` 2、`storage_stage0_tombstone_no_resurrect` 2、
  `session_fold_rebuild` 8、`session_rebuild_compare` 5。`cargo check --all-targets` 零警告；
  `npm run size:check` 未新增违规（`sqlite_session.rs` 6266 ≤ 基线 6276）。
- **真实库现状（事故副作用，如实记录）**：`~/.local/share/PonyAgent/sessions.db` 已按本命令完成一次迁移，
  现为 780MB / `freelist=0` / `auto_vacuum=1`，`integrity_check=ok`，数据行数未变。

## 遗留（已登记 backlog）

- 折叠吞吐本身（≈100MB/s）未变：大库仍拿不到「日志真源」视图，只是不再为它白付秒级代价。
  彻底解需要「首帧后后台折叠 + 视图失效协议」，`B048`。
- 维护项的修复结果仍是内存态（既有契约）：每轮启动都会重做约 0.57s 的同一批重建。要收敛需持久化
  修复结果或建立「已修」标记，均触及 index-only 轻量行契约，`B048`。
- `auto_vacuum=FULL` 是**一次性显式迁移**：未跑 `ponycli sessions vacuum` 的库仍会累积空洞。
  是否在首次启动自动迁移，取决于产品对「启动期破坏性维护」的取舍，`B049`。
