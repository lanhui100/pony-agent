# Agent Note: 会话日志文件切为写入真源（ADR 0099 阶段 1-B）

Status: implemented

## Problem

ADR 0099 阶段 0 交付后，会话真源仍是「一个 SQLite 库装所有会话」。由此派生的争用与
版本问题无法优雅解决：

- 全库只有一支笔 ⇒ 必须引入 daemon / 租约 / CAS 一整套机制，而阶段 0 两轮三路对抗审查
  共抓出 10 条 BLOCKING，每轮都能找到新的未守门落盘路径（单行写无 `WHERE version`、
  `INSERT OR REPLACE` 重置版本、`BUSY_SNAPSHOT` 静默丢弃等）。
- 混版本部署下 `events.schema_version` 是单值，旧版本遇新版**拒绝打开整库**。
- 未来形态（多 agent 并行、桌面连远端 Linux agent、任务分派）下，"笔归谁"这个问题本身
  不该存在。

根因判断：**不是介质，而是写单元粒度**。把写单元从「全库」降到「单会话」，争用结构性消失。

## Decision

1. **真源 = 每会话一份 append-only 日志文件**（`<root>/--<cwd>--/<encoded-id>/session.jsonl`），
   header 首行自描述版本，四态分类 `Current | MigrationRequired | Unsupported | Malformed`，
   append + fsync，失败回滚到先前字节长度，崩溃只丢撕裂尾行。
2. **SQLite 降级为可重建索引**：查询照旧走 SQL，但索引损坏 ≠ 数据丢失。
3. **写路径接线（阶段 1-B，本笔记）**：`persist_command` / `persist_commands_batch` 中
   **先追加写日志并 fsync，成功后再写 SQLite**；日志写失败 ⇒ `PersistCommandOutcome::Failed`
   且 SQLite 一行不改（真源优先，不静默成功）。`RemoveSession` 在 SQLite 提交成功后才删日志目录。
4. **日志事件信封** `{"seq","ts","cmd"}`，`cmd` 为自描述的 `PersistCommand` JSON —— 真源无损
   承载现有命令流，可据此重建状态。
5. **文件锁统一为 `std::fs::File::try_lock`**（跨平台；Unix 即 flock，Windows 即 LockFileEx），
   删除 `#[cfg(unix)] libc::flock` / `#[cfg(not(unix))]` 双胞胎 ⇒ 只剩一条经编译与测试验证的路径。
6. **每会话长命写者句柄**：独占锁只抢一次，避免「每批重开重抢锁」在同进程多线程下的瞬时
   `Locked` 竞态（实测会让 `flush_events_concurrent_seq_no_duplicate_no_gap` 偶发失败）。

## 关键取舍（记录以备复核）

- **cwd 分组段用进程当前目录**：backend 层拿不到会话 cwd；该段按设计是「可读、可损失」的
  展示标签，唯一性由单射 session id 保证，故不为它新增跨层契约。
- **日志根目录 = `db_path.parent()/session-logs`**：复用 `attachments/` 同款既有约定，不硬编码路径。
- **不做双写灰度**：用户裁决「直接切写入」（无重要历史数据），因此没有 7 天 diff 比对窗口，
  回归保障完全由测试承担（见下）。

## Verification

- `cargo test -p pony-agent-core --test session_log_wiring` → 7 passed / 0 failed（Lead 独立复跑）
- `cargo test -p pony-agent-core --test session_log_store` → 6 passed / 0 failed（含 32 例撕裂尾行、
  10 例目录逃逸）
- `cargo test -p pony-agent-core --lib` → 1379 passed / 0 failed
- `cargo check -p pony-agent-core` → 0 warning
- 反向守门：日志根只读 / 根路径是文件 ⇒ `Failed`，且 `sessions.db` 与 `-wal` 的 sha256 逐字节相同
- 规模门禁：`store.rs` 5692 ≤ 5704、`sqlite_session.rs` 6238 ≤ 6276；`check-file-size` 本轮唯一
  红项为 `tools.rs` 14841 > 14816，属**另一 lane 的并发改动**，与本改动无关


## 关联与取代关系

- **上位决策**：[ADR 0099](../../../docs/decisions/0099-session-storage-event-sourced-refactor.md)（D1/D2 于 2026-11-08 修订为「每会话一份日志文件为真源」）。
- **直接触发**：[2026-11-08-tui-enter-key-drift-and-session-row-heal.md](../bug-fix/2026-11-08-tui-enter-key-drift-and-session-row-heal.md) —— 多实例整包写盘删除他人会话、`pony resume` 谎称无历史。该笔记的应急修复（「整包写盘只写不删」+「resume 按 id 从规范化表自愈重建」）**在本笔记落地后成为过渡措施**：
  - 「不删」不再是约束——写单元已降到单会话，不会再触碰他人的会话；
  - 「自愈重建」不再是必需——文件即真相，SQLite 缺行由阶段 2 的 `rebuild_index` 补齐。
  两者**保留不回退**（成本为零、可继续兜住历史遗留），但不再是唯一的正确性来源。
- **阶段 0 记录**：[ADR 0099 修订段](../../../docs/decisions/0099-session-storage-event-sourced-refactor.md) 记录了版本 CAS 移交阶段 1 的根因（10 条 BLOCKING）。

## Alternatives considered

1. **旁路双写 + 7 天 diff 比对后再切读**（原建议）：安全性最高、可回滚，但多写一份、周期长。
   用户以「无重要历史数据」明确否决，改为直接切写入，风险面由测试与「失败即整体失败」语义兜住。
2. **保留 `libc::flock` + `File::try_lock` 双分支**：能表达平台差异，但非 Unix 分支在本机
   **无法编译验证**，等于留一个未测死角。统一到 std 跨平台 API 后该死角消失，且 Windows 语义
   仍是 LockFileEx。
3. **每次 append 重开文件并重抢锁**：实现简单，但在同进程多线程下存在瞬时 `Locked` 竞态
   （已实测偶发失败），且每次都要重扫文件长度；改为长命句柄后两者同时解决。
4. **日志事件直接序列化 `SessionState` 快照**：实现最省，但会退化成「文件版整包覆盖」，
   丢掉 append-only 的崩溃韧性，且与阶段 2 的 fold 重建目标冲突。否决。
5. **把日志写入放在 SQLite 提交之后**：可保持现有错误语义，但会造成「SQLite 有、真源没有」
   的分裂态，重建索引时数据凭空消失。否决（真源必须先行）。
