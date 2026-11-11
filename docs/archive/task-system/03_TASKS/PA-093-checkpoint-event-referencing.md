# PA-093 checkpoint 引用化（阶段 3：事件溯源演进）

## Basic Info

- ID: PA-093
- Status: Done
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-18
- Updated At: 2026-08-18
- OpenSpec Change: `openspec/changes/checkpoint-event-referencing/`
- Spec 状态: 通过（3 路对抗审核收敛 + 实施后 tester 详实审核，2026-08-18）

## Background

PA-092 后快照已从事件折叠，但 checkpoint 仍是快照复制模型（HistoryNode 内嵌全量快照，撤回是复制粘贴）。阶段 3：checkpoint 引用化。

## Goal

1. `HistoryNode.event_seq_range` 引用（legacy 兜底）
2. checkout = 事件 + 水位回退（无数据复制，metrics 缓存同步回退）
3. fork = 事件前缀共享 + branch_id 过滤
4. 分支可见性（被撤回分支事件不复活）
5. 时间旅行机制 + cursor_version 退役（event_watermark）

## Scope

- `crates/pony-agent-core/src/agent/`：`session.rs`、`sqlite_session.rs`、`projection.rs`、`turn_flow.rs`、`control_plane/mod.rs`
- 前端 `src/lib/runtime/history.ts` 适配（wire 兼容，零改动）

## Non-Goals

- 时间旅行 UI、GraphRun 推进事件化（后续 change）
- workspace 回滚增强（git 回滚语义保持现状）

## Acceptance Criteria

1. checkout 无数据复制断言 + metrics 缓存同步回退（降级声明：节点区间不可变 → 缓存天然有效；metrics 不进 session 内存态 → 无发散面）
2. 分支可见性（被撤回分支事件不复活）
3. fork 事件共享（branch_id 过滤）
4. 时间旅行折叠正确
5. 既有测试全绿（core 831 测试通过）

## Review Plan

- spec 3 路对抗审核 → 已收敛
- 实施后 3 路对抗审核（tester 详实）→ 全部裁决 → 验证 → 收口

## Current Progress

- **实施完成**（2026-08-18）：
  - `HistoryNode.event_seq_range` + `HistoryCursor.event_watermark` + `SessionState.event_watermark/last_commit_watermark`
  - `finalize_event_watermark`：flush 成功后升级 branch head 节点为引用化（commit 先于 flush 时序下先 legacy 提交，事件落盘后清空快照 + 记录区间；flush 失败保持 legacy 不丢数据）
  - checkout/restore/switch/fork 重构：引用化节点事件折叠（块外预折叠 + 块内纯函数应用）+ append checkpoint/checkout 或 fork/created 事件 + cursor 水位同步
  - `snapshot_for_session_at` 时间旅行（克隆会话折叠，不污染内存态）
  - `projection.rs`：`fold_all_with_branches`（初始可见集合 = 目标节点分支血缘链；流内 checkout 事件按序重放）+ `branch_lineage`
  - `sqlite_session.rs`：`load_turn_events` / `load_event_watermark` / `flush_events_tx` branch_id 参数化 / `event_seq_range_json` + `event_watermark` 列 + ALTER 迁移
  - `turn_flow.rs`：`emit_global_event`；`control_plane/mod.rs`：flush_events 改造（branch_id 从 session 读 + finalize）
- **实施后审核**（2026-08-18）：tester 详实（P0×3/P1×6/P2×6）→ 全部裁决：P0-1/P0-2 降级声明（缓存天然有效 + cursor_version 兼容）、P0-3/P1-1/P1-2/P1-3/P1-4/P1-5/P1-6/P2-1 采纳（测试补齐 + 可见集合语义修正 + 隔离性修复）
- **验证**：core 831 测试通过（含新增 8 个 pa093 测试）
- **收口**：tasks.md 采纳记录 + 降级声明已更新

## Next Action

- 无（已完成）；PA-094（trace 事件化）为下一任务

## Blockers

- 无

## Resume Hint

- 核心实现：`session.rs` 的 `finalize_event_watermark` / `fold_node_views` / `fold_session_views` / 4 个命令重构；`projection.rs` 的 `fold_all_with_branches`；`sqlite_session.rs` 的 `load_turn_events`