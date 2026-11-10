# Agent Note: 会话库装载成本收口与库体回收（ADR 0118 实施记录）

Status: implemented

- 日期：2026-11-10
- 决策真源：[`docs/decisions/0118-session-load-cost-and-store-reclaim.md`](../../../docs/decisions/0118-session-load-cost-and-store-reclaim.md)
- 交付面：`pony-agent-core`（会话装载路径）、`ponycli`（新增 `sessions vacuum`）

## Problem

ADR 0115 让首帧 50~100ms 后，**装载本身**仍 2.9~4.3s。三笔可归因成本（实测）：

1. `fold::fold_read_view` 的 800ms **时**预算超时语义是「整轮丢弃」⇒ 真实库 `session_fold_slow:
   elapsed=880~1050ms folded=10~13 skipped=141~144`，折出来的结果一行没用上，紧接着索引投影又把
   153 个会话全量读一遍。
2. `read_traces_normalized` 的 legacy 兜底把整张 `session_turn_traces`（222MB / 645ms）读完再逐会话
   `or_insert` 丢弃；真实库上「legacy 独有」会话数 = 0。
3. 维护预算 deadline 取在索引回填**之前**，回填（1.5s，不受预算约束）把它吃光 ⇒ `history_rebuild`
   每轮入口即过期（`sessions=28`），24 个会话重启后历史恒空。

库体侧（`B046`）：1144MB 库里 445MB 是 freelist 空洞（`auto_vacuum=OFF` + 整块 DELETE），WAL 曾积 431MB。
先立反证：空洞**不拖慢**启动（同快照 A/B 首帧前读盘字节完全一致 3083MB）。

## Decision

1. **折叠字节预检**：`DEFAULT_READ_FOLD_LOG_BYTES = 32MiB`；`fold_read_view` 进循环前 `stat` 日志总量，
   超预算直接整轮降级（落 `session_fold_precheck`），不再「读满 800ms 再丢弃」。降级语义不变。
2. **trace 兜底收窄**：先读规范化侧（E7 主源），再只对「规范化侧给不出 trace」的会话逐会话回读 legacy；
   删除 `read_all_session_traces`（唯一消费者即此路径），新增 `commit::legacy_trace_session_ids()`。
3. **维护预算按阶段起算**：自愈拆为「索引回填（无预算）」+「`run_budgeted_startup_self_heal`（带 deadline）」，
   `load_store` 在回填后 `reset_startup_budget()`。
4. **库体回收改 FULL**：新增 `ponycli sessions vacuum [--dry-run]`（`inspect_session_store` 只读探针 +
   `reclaim_session_store_space`：维护锁 → `auto_vacuum=FULL` + `VACUUM` → `wal_checkpoint(TRUNCATE)`）。
   `remove_session` 零改动（FULL 自带回收）。
5. **写路径 checkpoint 阈值化**：小 WAL 走 `PASSIVE`；≥64MiB 才 `TRUNCATE`，busy 等待压到 50ms、
   抢不到立即退回 `PASSIVE`（回收失败不得变成保存失败）。
6. **维护子命令 `--help` 护栏**（事故现场）：`run_maintenance_subcommand_if_requested()` 见到 `--help`/`-h`
   一律让路；`sessions`/`config` 命名空间下打印完整用法并 `exit 0`，新增 `is_help_request()` + 2 例单测。
   事故经过：验证 `ponycli sessions vacuum --help` 时真的执行了破坏性维护，把真实库 1144MB VACUUM 到 743MB
   （`integrity_check=ok`、行数不变）。数据无损，但「读帮助产生写」本身是缺陷，且对 `rebuild-index` /
   `migrate-logs` 等所有维护子命令同源——本次一并收口。

## Alternatives considered

详见 ADR 0118「候选方案」：
- F1：保留部分折叠结果（**落选**：改降级契约，且折叠吞吐 ≈100MB/s、682MB 需 6.8s，比索引投影的 1.5s 贵 4 倍）；
  首帧后后台折叠 + 视图失效协议（**落选**：独立决策，本轮先停掉注定被丢弃的 IO）；
- F2：`NOT EXISTS` 单条 SQL（**落选**：判不了 `raw_json IS NULL` 占位行，语义会静默变窄）；
- F3：持久化游标（**落选**：只轮换「这轮修哪批」，不减少工作量；修复结果按既有契约是内存态）；
- B046：INCREMENTAL + `incremental_vacuum`（**实测否决**：整表 DELETE 后 1956 页空闲只收 1 页）；
  启动时自动 VACUUM（**落选**：把打开应用变成可能失败的破坏性维护）；只 VACUUM 不改模式（**落选**：会涨回来）。

## Consequences

- 首帧不回退：`ab-logs` 49/50/49ms、`ab-base` 52/55/52ms。
- 装载完成（`ab-logs`，同沙箱三轮）：2970/2897ms（冷启动 7805ms）；旧实现同沙箱 ≈4.0s。
- `ab-base`（无日志）默认 3510~3784ms；`PONY_STARTUP_SELF_HEAL_BUDGET_MS=0` 时 3007~3250ms
  ⇒ 维护项 +0.57s，换来 `history_rebuilt_from_turn_events: 24`（原先恒空）。净时间持平，内容不再丢。
- 库体（真实库副本）：1144MB → **697MB**、freelist 114048 → 0、`auto_vacuum=0→1`、5056ms；
  `integrity_check=ok`、行数不变；迁移后插入 4MB 再删：731.5 → 735.5 → **731.5MB**（自动回收）。
- 测试：`--bin pony` 119 passed；`--lib store_reclaim` 6 passed（含 WAL 阈值 + busy 回退）；
  `--test storage_stage0_recover_trace` 2 passed（新增 legacy 独有兜底守卫）；
  `--test session_read_from_log` 8 passed；`cargo check --all-targets` 零警告；`size:check` 无新增违规。
- 残余风险（靠 review）：折叠降级后大库长期只看索引投影（日志真源不生效，属既有事实但被显式化）；
  `auto_vacuum=FULL` 的写放大随「每次提交释放页数」增长（200MB 库单次删除 +4ms 实测，超大 blob 批量删除需再观察）。
- 遗留：后台折叠 + 视图失效协议（`B048`）、维护修复结果持久化（`B048`）、首次启动自动迁移取舍（`B049`）。
