# wave-p1-0107 交付说明（ADR 0107 P1：多进程写者）

分支 `wave/p1-session-lease`（worktree `/home/dm/pony-agent-worktrees/wave-p1-0107`），基线 = P0 分支 HEAD `e043f32f`
提交链：`7c8faeb1`（契约+风险登记）→ `bed8701c`（红相锚定）→ `9352ef19`（实施）

## 一、行为契约（D1–D5）

| ID | 契约 | 实现 |
|---|---|---|
| D1 | 另一进程持库级锁时，普通写入**不再降级只读** | `sqlite_session.rs::connection()` 删除 `acquire_single_instance_lock` 与 `query_only` 降级分支；统一 WAL + `busy_timeout=5000` + `ensure_schema` |
| D2 | 维护锁被占时，维护入口**显式拒绝** | 新增 `commit::acquire_maintenance_lock` → `MaintenanceBusy`；`fold.rs::rebuild_index_at_root` 非 `dry_run` 先取锁 |
| D3 | 两进程写**不同**会话都成功且进索引 | 多进程 WAL 串行写；不再有进程级全局写锁 |
| D4 | 两进程写**同一**会话：后者显式失败且可识别 | 日志层 `LogStoreError::Locked` → `PersistCommandOutcome::WriterBusy` → `SessionError::SessionWriterBusy { session_id }`，日志行 `session_writer_busy` |
| D5 | 日常写路径不依赖库级实例锁 | `try_recover_instance_lock` 删除；`is_read_only()` 收窄为 `storage_read_only` |

## 二、物理收据（`9352ef19`）

| 门禁 | 结果 |
|---|---|
| `p1_multiprocess_writers`（D1–D5） | **5 passed** / exit 0（红相基线为 D1/D3/D4/D5 FAIL） |
| `p0_log_first_durability` | 7 passed / **2 ignored**（C1/C6 按 P1 语义退役，见下）/ exit 0 |
| `cargo test --lib` | **1408 passed** / exit 0 |
| `session_log_wiring` + `session_read_from_log` + `session_log_store` + `session_event_carriers` + `pa174…` | 全绿 / exit 0 |
| `cargo check --all-targets` | exit 0，0 warning |

原文：`.dev-team/red-phase-p1-0107.log`、`.dev-team/green-phase-p1-0107.log`。

## 三、P0 测试的退役与改写（显式登记）

- **C1 退役**（`#[ignore]` + 原因）：P1 取消了「锁降级只读」状态，该用例的前提不复存在；
  日志先行的实质由 C2/C4 与 P1 的 D1/D3/D5 覆盖。
- **C6 退役**：降级实例不存在 ⇒「降级实例按批释放锁」失去对象；会话级单写者由 D3/D4 覆盖。
- **C7 改写构造**：不再用降级实例制造"索引必失败"，改用可写实例验证 R5 实质
  （零字节批不得自称 `LogPersisted`）；断言强度不变。
- C2/C3/C4/C5/C8/C9 **原样保留并全绿**（P0 的日志先行、失败域隔离、stale-epoch 预检/回滚不回归）。

## 四、已知风险（P1 引入的新并发面，已在契约登记）

| 风险 | 现状 | 处置 |
|---|---|---|
| R-A 旧 JSON 迁移 TOCTOU | `migrate_from_json` 仍是「先 COUNT 再导入」；两进程同时首启可能重复执行（幂等覆盖，不丢数据） | P1.1：迁移入口取维护锁或加完成标记 |
| R-B 并发首建表 | `CREATE TABLE IF NOT EXISTS` + `busy_timeout` 已可承载 | 绿相未出现失败；需长稳观察 |
| R-C WAL 检查点竞争 | SQLite 自身串行化 | BUSY 由既有 3 次退避兜底 |
| R-D 同会话冲突提示 | 已类型化 + 结构化日志 | TUI/桌面侧文案待接（P2） |

## 五、合并顺序

```
cd /home/dm/pony-agent
git merge --no-ff wave/p0-0107-log-first   # 先 P0
git merge --no-ff wave/p1-session-lease    # 再 P1
```
主工作区有他人未提交改动（`sqlite_session.rs` / `sqlite_session_commit.rs` / `store.rs` 同文件），合并前需先处理。
