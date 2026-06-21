# PA-058 拆分 session trace 存储并扩展定向持久化

## 状态
- Status: `Done`
- Priority: `P3`
- Owner: `Codex`
- Completed At: `2026-06-21`
- Dependencies: `SessionBackend` trait stability; `MemorySessionBackend` needs parallel trace-method implementation for test coverage

## OpenSpec Change
- 已归档：
  [2026-06-21-split-session-trace-storage-and-targeted-persistence](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-21-split-session-trace-storage-and-targeted-persistence>)

## Delta Spec
- 已归档：
  [session-trace-storage-and-targeted-persistence/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-21-split-session-trace-storage-and-targeted-persistence/specs/session-trace-storage-and-targeted-persistence/spec.md>)

## Canonical Spec
- [session-trace-storage-and-targeted-persistence/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/session-trace-storage-and-targeted-persistence/spec.md>)

## Spec 状态
- Proposal: `done`
- Spec: `done`
- Design: `done`
- Tasks: `done`

## 背景
本轮 session persistence 卡顿诊断已经收口出明确结论：turn 完成前的主卡顿不在前端渲染，也不在 trace clone，而在后端 `record_turn_trace -> save_to_backend -> SQLite write_full_store` 的全量持久化路径。

已完成的 Priority 1 / 2 近线修复包括：

- `record_turn_trace`、`annotate_turn_trace_terminal_event`、`append_turn_trace_hook_records`、`append_failed_turn`、`replace_long_term_memory` 已切到 `save_session_to_backend`
- `SqliteSessionBackend::upsert_session` 已支持单 session 行级 upsert
- 生产卡顿已从秒级下降到毫秒级

但当前 `turn_trace_history` 仍然内嵌在 `sessions.session_data` blob 中，这意味着：

- 单 session trace 增长仍会放大 JSON 序列化成本
- trace 写入与 session metadata 写入仍然耦合
- 历史图、control plane、monitor、reload 仍默认从同一 blob 读取 trace

因此需要立正式后续卡，把 Priority 3 从“修复补丁”升级为“结构性演进项目”。

## 目标
- 将 live session 的 `turn_trace_history` 从 `sessions.session_data` blob 中拆出为独立持久化单元
- 保持 `SessionSnapshot.turn_trace_history` 现有读面形状不变
- 在不破坏 `HistoryNode` 现有快照语义的前提下，为 trace 高频更新建立真正的增量读写路径
- 为后续更多 session-local 定向持久化扩展提供稳定 backend 接口

## 输出
- `split-session-trace-storage-and-targeted-persistence` OpenSpec change
- SQLite `session_turn_traces`（或等价） schema 设计
- 双读兼容策略：新 trace 表优先、旧 `session_data.turn_trace_history` fallback
- trace 热路径从 session blob 写入切换到独立 trace 存储的迁移计划
- 与 `HistoryNode.turn_trace_history` 的所有权与一致性说明
- 验证矩阵：reload / history restore / branch / monitor / control-plane

## 范围边界
- 本卡不改变前端 `SessionSnapshot.turn_trace_history` 消费 contract
- 本卡不在同一轮中重写 `HistoryNode` 数据模型
- 本卡不同时改造 attachment、source snapshot、session deletion 的全局元数据语义
- 本卡不把所有 session-local 路径一次性扩展到定向写；仅在 split 方案稳定后再逐步扩张
- 本卡不否定当前 Priority 1；它是在现有热修基础上的结构性演进

## 验收标准
- live session trace SHALL 能独立于 `session_data` blob 进行增量持久化
- `load_store` / `SessionStore::with_backend` SHALL 能从新 trace 存储组装出与当前相同的 `SessionState.turn_trace_history`
- 当新 trace 存储不存在时，系统 SHALL fallback 读取旧 `session_data.turn_trace_history`
- `record_turn_trace`、`annotate_turn_trace_terminal_event`、`append_turn_trace_hook_records`、`append_failed_turn` SHALL 不再要求重写整个 session blob 才能持久化 trace
- `snapshot`、`history checkout`、`restore_branch_head`、`fork_from_history_node`、`switch_history_branch` SHALL 保持现有语义
- 至少补齐以下验证：
  - live trace reload roundtrip
  - terminal envelope annotation roundtrip
  - hook trace append roundtrip
  - history restore / branch switch 不丢 trace
  - monitor / control-plane 仍能读取 trace-backed evidence

## 当前进展
- Priority 1 已完成：trace 热路径已改成单 session 定向 upsert
- Priority 2 已完成首轮安全扩展：`append_failed_turn`、`replace_long_term_memory` 已跟进
- 已完成后端性能定位：SQLite `write_full_store.session_upsert` 是全量写热点
- 已完成风险梳理：`turn_trace_history` 同时存在于 live session 与 `HistoryNode`，不能直接粗暴拆表
- 已完成双智能体方案审核，确认 P3 应作为独立结构改造推进，而不应混入本轮热修
- 已实现独立 `session_turn_traces` 表、dual-read / authoritative no-fallback、hot path trace-level mutation、组合写事务、branch/history trace materialization、`NotFound` 自愈与 trace 表 prune
- 已通过多轮并行智能体审核与全局验收，最终 `cargo test -p pony-agent-core --lib sqlite_session -- --nocapture` 15 passed

## 验收结果

### A. 独立 trace 持久化
- `session_turn_traces` 表已建立，trace 可独立于 `session_data` blob 持久化
- hot path trace-level mutation 已就位：upsert / terminal-event update / hook append
- `WriteSeparate` 下 session blob 剥离 live `turn_trace_history`

### B. 读面兼容与 dual-read
- `SessionSnapshot.turn_trace_history` 形状不变
- dual-read / authoritative no-fallback 已实现
- merge 顺序稳定

### C. 组合写事务
- session row + trace mutation 同事务提交
- `NotFound` 语义与自愈 `ReplaceAll` 路径

### D. Branch/History 正确性
- checkout / restore / fork / switch 后 trace materialization 正确替换
- branch restore SQLite 端到端回归通过

### E. Trace 表管理
- prune 支持，限制到 history cap
- session 删除时 trace 行同步清理
- 缺行 / 坏行安全降级

## 下一步动作
- 无（本卡已关闭）

## 当前卡点
- 无（本卡已关闭）

## 断点续跑提示
- 本卡已关闭；相关数据路径的后续扩展应另开新卡
- 参考 implementation：
  - [session.rs](</C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/session.rs>)
  - [sqlite_session.rs](</C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/sqlite_session.rs>)
  - [canonical spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/session-trace-storage-and-targeted-persistence/spec.md>)
