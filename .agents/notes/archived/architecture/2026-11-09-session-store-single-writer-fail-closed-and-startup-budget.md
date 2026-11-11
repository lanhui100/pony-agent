# Agent Note: 会话库单写者冲突 fail-closed、退出提示真实性、启动装载预算

Status: archived (原状态: superseded)

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0106-session-store-single-writer-fail-closed-and-startup-budget.md` 之归档快照，仅供溯源。

> **接替范围**：本文的「单写者 fail-closed / 第二实例拒绝启动」锁语义由
> [ADR 0107](0107-session-scoped-writer-lease-and-log-first-durability.md) §5 P1 取代（改为多进程可写 +
> `<db>.lock` 仅服务维护）；本文的**启动预算、`startup_meta` 轻量解析、退出提示真实性**等能力仍有效并被保留。
> 另：本文引入的进程级预算 `OnceLock` 缺陷（长驻进程后续装载永久跳过历史重建）已在 0107 实施中修复。

## 背景

2026-11-09 现场：用户在一个终端里运行 `pony` TUI，另开一个终端执行退出提示给出的
`pony resume pa_01a1bfd4-…`。第二个实例抢不到 `sessions.db.lock`，按既有设计「降级只读」，
但**界面无任何提示**：它照常接受用户输入，每轮在提交边界失败。

现场日志（`~/.pony-agent/tui-logs/tui-runtime.log`，本次窗口内累计 211 条同类记录）：

```
[pony-agent][session] single_instance_lock_denied: …/sessions.db.lock 已被其他进程独占；本实例降级只读
[pony-agent][session] commit_boundary_failed: begin immediate tx: attempt to write a readonly database
[pony-agent][session] save_to_backend 失败：commit_boundary_failed（数据可能丢失）
```

只读取证（真实库，`mode=ro`）：`pa_01a1bfd4-…` 在 `sessions` / `normalized_sessions` /
`normalized_messages` / `turn_events` **四张表行数全为 0** —— 提示里的 id 是一个从未落库的幽灵会话。
用户按提示恢复，只会得到「尚无历史记录，已沿用该 id 开始新会话」，于是**提示 → 恢复 → 仍然没有历史**
形成闭环。同型事故在 PA-174 已出现过一次（`turn_events` 是唯一幸存数据面），说明这不是偶发。

同时该库已进入「启动即慢」区间：`sessions.db` 1.2 GB（+ WAL 431 MB）、`session-logs` 535 MB / 143 会话。
启动日志证据：

```
session_fold_slow: elapsed=1470ms budget=800ms folded=10 skipped=133
index_projection_filled: 143 个轻量索引行由 normalized_* 投影补全
history_rebuilt_from_turn_events: 24 个投影空会话由事件日志重建历史
```

读路径在三处**无预算地全量做功**：`load_index_store` 的逐会话 trace 查询（N+1）、
`fill_index_only_rows_from_index_projection` 的全表投影装载、
`rebuild_history_from_turn_events_when_projection_empty` 的逐会话事件流读取；
`pony resume` 另外在 `resolve_resume_target` 与 `build_control_plane` 各做一次整库装载。

三条缺陷的共同点：**把「不能写」与「什么都没发生」混为一谈**——既不告诉用户，也不阻止用户，
最后把用户输入静默丢弃。

## 候选方案 / Alternatives considered

**A. 保持只读降级，但加显式横幅 + 拒绝轮次提交**（当时的默认方向）
优点：可只读浏览历史；改动小。
落选原因：只读浏览在这个产品里没有独立价值（历史就在同一会话里，切到另一个进程看不解决任何问题），
却引入「界面能输入但不能提交」的二等状态，需要额外状态机与文案；而真正的问题——用户以为自己在
正常工作——只要还允许进 TUI 就仍然容易误解。另外 read-only 实例还会跳过 ask store，Ask 面板行为
与可写实例分叉，可测性差。

**B. 抢不到锁时轮询等待，持有者退出后自动接管**（"等一下就好"）
优点：用户无感。
落选原因：`pony` 的持有者常常是**另一个长期打开的 TUI**，等待没有上界；把「等待另一个人类关窗口」
做成隐式交互会让启动时长不可预测，也让超时后的行为重新回到 A 的两难。

**C. 校验锁文件 pid 存活并强行接管**（当作 stale lock 处理）
落选原因：`flock` 由内核在进程死亡时自动释放，**不存在**需要人工清理的 stale lock；
真正持有者必然活着。按 pid 判活再删除/接管只会引入 TOCTOU 竞态，并把「单写者」承诺变成建议。

**D（选中）锁冲突 fail-closed + 持有者身份可见 + 提示真实 + 启动预算**
1. **锁文件携带持有者身份**：抢到锁后 best-effort 写入 `pid` / 启动时间 / 命令行；抢不到时读回并写进
   结构化日志，新增三态查询 `single_instance_lock_status()`（`Writable` / `HeldByOther{pid,command,since}` /
   `Unavailable{reason}`），对同进程已持有的情况查进程内登记表（避免 `flock` 以 open file description
   记账导致的自锁）。
2. **TUI 启动 fail-closed**：`pony` / `pony resume` 在创建任何 store、进入 raw mode 之前预检；
   `HeldByOther` ⇒ 向真实 stderr 打印持有者 pid / 命令行 / 启动时间与处置建议后非零退出。
   `--version / --help / --doctor / update` 不预检。
3. **退出提示只对已落库会话承诺**：`继续该会话请运行：pony resume <id>` 仅在库内确有该会话时打印；
   否则明说「本次会话未落库（没有可恢复历史）」；运行期若发生过保存失败，同时把「数据可能丢失」告知用户。
4. **启动装载有预算**：`pony resume` 的预解析改用轻量元数据查询（`session::startup_meta` 的一条只读
   SELECT，不再整库装载）；启动期修复（trace 投影补全、事件流重建、trace 缓存重建）改为受**进程级
   单条 deadline** 约束（`PONY_STARTUP_SELF_HEAL_BUDGET_MS`，默认 1200ms，`0` = 全跳过），超预算跳过
   剩余会话并打一条结构化日志，语义是「本轮按索引视图，下轮再补」。
   **索引投影回填例外、不受预算约束**：它是「日志 fold 降级 ⇒ 仍给出完整索引视图」这条既有契约的落点，
   跳过它会把几十个会话读成空壳（不是提速，是读错）。
5. **空壳会话不得重算派生字段**：停写 blob 后索引里只剩 `__index_only` 轻量行，内容不在索引里；
   若照旧 `refresh_session_metadata` 重算，`title` 会被打回默认标题、`turn_count` 打回 0，再随启动回写
   **永久降级**。启动修复因此只在 `history` / `turn_trace_history` 至少一边非空时重算，且启动期整包
   回写改为**可归因**（`startup_repair_commit: reasons=…`），不再无因由地每次启动写全库。

选中理由：把不可判定的状态（"我以为我在工作，其实全都丢了"）从产品里删掉；诊断信息落到用户能看见的
出口；启动耗时从"随库增长"变成"有上界"。

## 决策

TUI 在会话库单写者冲突时**拒绝启动**并给出持有者可执行诊断；退出提示只承诺真正可恢复的会话；
启动装载（含自愈）统一受预算约束，超预算降级为索引视图而不是无限期阻塞。

## 影响

- 桌面端与 TUI **不能同时**以可写身份运行同一 `sessions.db`：TUI 会拒绝启动并提示先关闭另一个实例。
  这是 ADR 0099「单写者」承诺的落实，不再是"降级后各写各的"。
- 只读诊断能力保留在库层（`SqliteSessionBackend` 的只读句柄仍可用，`ponycli` / 脚本不受影响），
  只是不再通过"可交互 TUI"暴露。
- 新增本地取证日志（无新增网络出口、不含正文与凭据）：
  `single_instance_lock_denied … holder pid=… cmd=… since=…`、
  `startup_self_heal_skipped: component=…`、`startup_repair_commit: reasons=…`、
  `persistence_evidence_fallback`。
- 新增一个**本地调参**环境变量 `PONY_STARTUP_SELF_HEAL_BUDGET_MS`（默认 1200，`0` = 跳过启动自愈）；
  非法值回退默认。它不是遥测出口。
- 代码落点：锁原语与三态查询收敛到 `agent/instance_lock.rs`；`pony resume` 预解析进入
  `agent/session/startup_meta.rs`；启动期有界修复进入 `agent/session/startup_repair.rs`
  （预算真源与空壳保护都在那里）；TUI 侧预检与退出证据在 `bin/tui/persistence_status.rs`。
- 锁文件格式变化（`<db>.lock` 从空文件变为三行身份信息）：向后兼容——旧实例只做 `open+flock`，
  不解析内容；读不到身份时新实例降级为 `holder=unknown`。
- 实测（同一份真实数据副本：`sessions.db` 1.2 GB + `session-logs` 535 MB、143 会话，tmux 110×36，
  release 构建，冷启动到首帧，同一副本上新旧二进制背靠背各跑 2–3 次）：
  旧 **7845–8625 ms** → 新 **4719–4980 ms**（≈ −43%）。构成变化：
  - 启动期整包回写（`save_to_backend` 1982–2330ms）→ **0**（派生修复留在内存）；
  - 索引装载的逐会话 legacy trace 读取（143 次）→ **0**（轻量索引行不读 legacy trace 表）；
  - 自愈（trace 投影 / 事件流重建 / trace 缓存重建）→ 受 1200ms 预算约束，超预算记
    `startup_self_heal_skipped`。
  - 仍未消除（已登记后续）：日志 fold 在超预算时整轮丢弃（≈850ms 白做功）、
    索引投影回填里的 `load_store_normalized` 仍全表读 trace（205MB）。
- 后续待办（不在本篇范围）：真实库的 1.2 GB / WAL 431 MB 属写放大历史债（ADR 0099 G1），
  需要独立的一次 `VACUUM` + `wal_checkpoint(TRUNCATE)` 维护窗口，不在启动路径上自动做。

