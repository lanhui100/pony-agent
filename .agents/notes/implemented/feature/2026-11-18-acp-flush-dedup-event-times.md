# Agent Note: ACP 委派 flush 并发去重 + 日志 eventTimes 解码

Status: implemented

工单：B239。影响文件：`agent/control_plane/mod.rs`、`agent/control_plane/event_buffer.rs`（新）、
`agent/control_plane/tests/tests_flush_dedup.rs`（新）、`agent/session/fold.rs`、
`tests/b239_log_rebuild_event_times.rs`（新）。

## Problem

1) 终态/drain flush 的「快照→提交→移除」没有 per-(session,turn) in-flight 标记：两路并发可在
   任一方移除前快照同一批 → 同一批事件被重复提交；
2) `finalize` 原挂在每次 `commit_event_batch` 之后，step 边界 hop 会把上一轮节点区间**提前冻结**；
3) 持进程级缓冲锁跨越 `sessions.write` 提交，锁范围过大；
4) `flush_events` 命令的 `eventTimes` 只编码不解码（fold 忽略该键）：日志重建 DB 时 C6.2
   逐事件时间语义不可复原（全部塌缩到批首时间）。

## Decision

- 新增 `EventBufferState{ batches, in_flight, terminal_pending }` 独立模块（锁纪律：缓冲锁只覆盖
  快照/收口两个短临界区，绝不跨 `sessions.write`）：
  - flush 先在锁内标记 in_flight 再快照；并发的第二路见 in_flight 直接跳过（幂等）；
  - 提交成功后只摘除**快照覆盖的前缀**（提交期间新入队事件保留尾部，不吞事件）；提交失败释放
    in_flight、保留缓冲；
  - `finalize` 仅在**终态批**提交成功后执行一次（终态事件先于该轮 append_turn 落盘）；step 边界
    flush 不再 finalize；
  - 提交顺序按 key 排序稳定化。
- `fold` 侧新增 `decode_event_times`：逐位取正数时间；缺位回落本批第一个正数；旧日志整键缺失
  回落该日志行批首时间。`FoldedSession` 新增 `event_records: Vec<FoldedTurnEvent>`（turn_id +
  event + created_at_ms），不写 0、不塌缩。
- 不在 `rebuild_index` 强写 `turn_events`（日志不含事件 seq，强写会丢 legacy 行/破坏既有重建
  契约）：本次只交付可复原的重建记录；「重建直接回填 turn_events」如需另立工单。

## Alternatives considered

- **A. 全局锁包住快照+提交（简单但串行）。** 落选：把网络往返/DB 提交纳入全局锁，flush 争用
  会拖慢 step 边界增量落盘；in-flight 键能在锁外提交下仍保证幂等。
- **B. 提交后清空整个键的缓冲。** 落选：提交期间新入队的事件会被吞；只摘除快照前缀才不丢尾。
- **C. 每次 flush 都 finalize。** 落选：step 边界 hop 提前冻结上一轮节点区间，正是本卡根因。
- **D. 日志重建直接写 turn_events。** 落选：日志不带事件 seq，重写会破坏 legacy 事件行与既有
  重建契约（需另立工单）。

## Verification

- `cargo-lane test --test acp_delegate_persistence_contract` 3 passed（C6.1 含 without duplicates）；
- `cargo-lane test --lib -- concurrent_flushes` 1 passed：Barrier 对齐两路 flush，提交分布严格
  `[0,3]`（恰好一路提交整批），DB 事件类型顺序唯一 turn/start→user/message→assistant/message，
  二次 drain 返回 0 且行数仍 3（无 sleep/无 ignore）；
- `cargo-lane test --test b239_log_rebuild_event_times` 3 passed（逐事件不塌缩 / 旧日志缺键回落
  批首 / 部分缺位回落批首）；
- `cargo-lane check -p pony-agent-core --all-targets` exit 0；本任务文件 size:check 0 新增。
