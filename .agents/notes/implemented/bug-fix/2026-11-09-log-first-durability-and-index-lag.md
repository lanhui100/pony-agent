# Agent Note: 日志优先持久化与索引滞后（会话库多窗口止血）

Status: implemented

## Decision

P0 波次（ADR 0107 §5）已落地：只读降级实例仍写会话 JSONL 真源；日志已落盘后的索引失败降级为
`PersistCommandOutcome::LogPersisted`（内存 live 历史照常提交）；stale-epoch 命令类型化拒收
（`StaleEpoch`），写日志前 fail-closed 预检、事务拒收时持锁回滚本批字节且校验偏移未被推进。

## Alternatives considered

- **只做 ADR 0106 fail-closed（拒绝第二实例）**：零新增并发面，但直接拒绝多窗口/多智能体，不满足需求。
- **先做 P1 会话级租约再止血**：一步到位但改动面覆盖锁原语/后端/CLI/TUI，止血周期过长。
- **仅"索引失败不回滚内存"、不动日志**：日志不写则崩溃/重启后历史仍丢，trace 无法重放。
- **tombstone + fold 过滤代替截断**：无截断风险，但需同时改 fold 与日志格式版本，超出止血闭包。

## Consequences

- 多窗口下历史不再丢失；索引故障只影响新鲜度。
- 代价：降级实例每批重开 writer 全扫该会话日志（10MB≈16.8ms，线性）；epoch 预检每批一次本地 SELECT；
  预检读失败使该批整轮失败（fail-loud）。
- 残留（P1/P2 承接）：同会话两实例并发写仍一方失败；回滚抢不到锁时 stale 批残留（可观测）；
  `persist_events` 仍把 `LogPersisted` 当非成功（与改动前一致）；索引滞后无 UI 提示。
