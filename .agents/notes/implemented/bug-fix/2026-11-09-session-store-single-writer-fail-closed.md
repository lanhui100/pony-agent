# 会话库单写者冲突 fail-closed + 退出提示真实性 + 启动装载预算

Status: implemented

- 日期：2026-11-09
- 关联：ADR 0106 / ADR 0099（单写者与事件溯源）/ ADR 0094（resume 自愈）
- 触发：用户现场「`pony` 启动慢」+「`pony resume <id>` 报会话尚无历史记录、未能启动」

## 现场证据（只读取证，未改真实库）

1. `fuser -v ~/.local/share/PonyAgent/sessions.db.lock` ⇒ 持有者是另一个仍在运行的 `pony` TUI 进程
   （用户开了第二个终端）。第二个实例按既有 E5 设计降级只读，但**界面对此零感知**：
   `~/.pony-agent/tui-logs/tui-runtime.log` 内累计 211 条 `single_instance_lock_denied`。
2. 幽灵会话 `pa_01a1bfd4-ebe5-75b1-8600-b9732cd642d4` 在 `sessions` / `normalized_sessions` /
   `normalized_messages` / `turn_events` 四表**行数均为 0**；退出提示仍打印 `pony resume <id>`。
3. 启动：`sessions.db` 1.2 GB + WAL 431 MB、`session-logs` 535 MB / 143 会话；
   启动日志 `session_fold_slow elapsed=1470ms budget=800ms`、`index_projection_filled: 143`、
   `history_rebuilt_from_turn_events: 24`。

## Decision

1. **锁持有者身份可见**：`<db>.lock` best-effort 记录 `pid` / 启动时间 / 命令行；抢不到锁时读回并写进
   `single_instance_lock_denied holder pid=… cmd=… since=…`；新增三态查询
   `single_instance_lock_status()`（同进程已持有 ⇒ `Writable`，避免 `flock` 自锁）。
2. **TUI fail-closed**：`pony` / `pony resume` 在创建 store、进入 raw mode 之前预检，
   `HeldByOther` ⇒ 真实 stderr 打印持有者诊断并非零退出；取消「静默降级为可交互只读」。
3. **提示真实性**：`pony resume <id>` 提示只在库内确有该会话时打印；否则明说未落库；
   运行期保存失败同时告知「数据可能丢失」。
4. **启动预算**：`pony resume` 预解析走 `session::startup_meta` 的一条只读 SELECT（不再整库装载）；
   启动期修复受**进程级单条** deadline 约束（`PONY_STARTUP_SELF_HEAL_BUDGET_MS`，默认 1200ms，
   `0` = 全跳过），超预算跳过并打 `startup_self_heal_skipped: component=…`，语义为「本轮按索引视图，
   下轮再补」。**索引投影回填不受预算约束**：它是「fold 降级仍给完整索引视图」这条契约的落点
   （`session_read_from_log::fifty_logged_sessions_load_within_budget_and_degrade_keeps_the_index_complete`
   守着它），跳过它会把几十个会话读成空壳。
5. **空壳会话不得重算派生字段**：停写 blob 后索引里只剩 `__index_only` 轻量行，内容不在索引里；
   旧逻辑会 `refresh_session_metadata` 把 title 打回默认、turn_count 打回 0，再随启动整包回写
   **永久降级**（与第 4 条的预算跳过叠加时必然触发）。现在只在 history / trace 至少一边非空时重算，
   trace 缓存重建改为逐会话 `AppendTrace`（不再因「重建过」整包提交），整包回写前记
   `startup_repair_commit: reasons=…` 以便归因。

## 实测（同一份真实数据副本，tmux 110×36，release）

- 改前：冷启动到首帧 **7845–8625ms**；启动日志 `session_fold_slow 834–859ms` +
  `commit_boundary_committed label=save_to_backend session_count=143 elapsed_ms=1982–2330`。
- 改后：**4719–4980ms**（≈ −43%）；`startup_repair_commit` 消失（改为 `startup_repair_deferred`），
  `session_turn_traces` 逐会话读取消失。
- 仍存（后续）：fold 超预算整轮丢弃 ≈850ms；索引投影回填的 `load_store_normalized` 仍全表读 trace。

## 机器验收回执

- `cargo check --lib --bins --tests` exit 0（无 warning）。
- `cargo test --lib`（`agent::session::` 162 项、`agent::sqlite_session::` 205 项经两轮分组执行）全绿。
- 集成：`session_read_from_log`(8) / `session_stop_blob_write`(7) / `session_fold_rebuild`(8) /
  `tui_persistence_truthfulness`(93+) / `tui_resume_cli` 全绿。
- 现场行为（真实 pty 采集）：
  - 第二实例（真实 HOME，锁被运行中的 TUI 占着）→ 打印持有者诊断并 **exit 1**；
  - 未落库 id 退出提示：旧二进制 `继续该会话请运行：pony resume pa_ghost_old`；
    新二进制 `本次会话没有可恢复历史：… 在会话库里没有可用记录（未落库或历史为空），无法续接。`
- 已知红灯（**非本轮引入**）：`session_row_heal::resume_recovers_session_from_normalized_rows_when_legacy_row_is_missing`
  在 HEAD（`c4a5bd81` 干净工作树）上同样失败——ADR 0099 把读路径切成「日志为真源」后，
  「legacy 行缺失 ⇒ 列表里看不到」这条**测试前提**已不成立。本轮把它改为断言**索引原样视图**
  （`load_index_store`）里没有该行，测试恢复绿。

## 为什么不是「只读降级 + 横幅」

只读 TUI 没有独立价值（历史就在同一会话里，换进程看不解决任何问题），却引入「能输入不能提交」的
二等状态，还要与 ask store 跳过逻辑对齐；而事故的本质是用户以为在工作、实际全丢。见 ADR 0106
候选方案 A/B/C 的比较。

## Alternatives considered

- **A. 只读降级 + 显式横幅 + 拒绝轮次提交**：改动小、能浏览历史；但引入二等交互状态与分叉的 Ask 面板
  行为，用户仍容易误以为会话在保存。落选。
- **B. 抢锁失败后轮询等待持有者退出**：用户无感；但持有者常是长期打开的 TUI，等待无上界、
  启动时长不可预测。落选。
- **C. 按锁文件 pid 判活并强行接管**：`flock` 在进程死亡时由内核释放，不存在需要人工清理的
  stale lock；判活再接管引入 TOCTOU 竞态。落选。
- **D. fail-closed + 身份可见 + 提示真实 + 启动预算**：选中（见 ADR 0106）。

## 未做 / 后续

- 真实库 1.2 GB / WAL 431 MB 属写放大历史债（ADR 0099 G1），需要独立维护窗口做
  `VACUUM` + `wal_checkpoint(TRUNCATE)`，不在启动路径上自动做。
- 桌面端与 TUI 并发写同一库从此会被拒绝：这是单写者承诺的落实，使用者需知悉（README/ADR 0106 影响节）。
