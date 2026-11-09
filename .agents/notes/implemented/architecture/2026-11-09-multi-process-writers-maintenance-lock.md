# Agent Note: 会话库多进程写者与维护锁分离

Status: implemented

## Decision

日常写路径不再获取 `<db>.lock` 进程级独占锁；多进程写同一 SQLite 索引改由 WAL +
`busy_timeout=5000` + 短事务 + BUSY 三次退避串行化。库级独占锁收敛为**维护锁**
（`commit::acquire_maintenance_lock`），只服务 `rebuild-index` / migrate / VACUUM 等
破坏性维护，抢不到即 `maintenance_lock_busy` 拒绝。同一会话的并发写由会话日志 `flock`
保证单写者，冲突类型化为 `SessionError::SessionWriterBusy`。`is_read_only()` 收窄为
仅维护/诊断句柄（`new_read_only` / `--dry-run`）。

## Alternatives considered

- **保留全局锁但改成阻塞等待**：实现最省，但把「等另一个人类关窗口」做成隐式交互，启动时长不可预测。
- **会话级 flock 租约（每会话一把锁 + epoch fencing）**：语义最精确，但需要新租约表与迁移，改动面覆盖 TUI/桌面/CLI；本轮先用「会话日志 flock 天然单写者」达到同等效果。
- **集中式存储服务**：彻底解决多端并发与远程，但对当前单机多窗口属过度工程，且推翻本地日志真源。

## Consequences

- 多窗口/多智能体可同时写索引；同会话冲突只影响该会话，且可识别、可提示。
- 代价：多进程写同一库时依赖 SQLite BUSY 退避（3 次、50/100/200ms），高争用下可能整批失败
  （失败时日志已落盘 ⇒ `LogPersisted` 可恢复，不丢历史）。
- 遗留：旧 JSON 迁移仍是「先 COUNT 再导入」，两进程同时首启可能重复执行（幂等覆盖，不丢数据），
  已登记为 P1.1。
