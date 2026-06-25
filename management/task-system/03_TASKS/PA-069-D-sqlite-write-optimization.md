# PA-069-D SQLite 写路径优化

## Basic Info
- ID: PA-069-D
- Status: Ready
- Priority: P2
- Owner: @agent
- Created At: 2026-06-25
- Estimated Effort: 2h

## Goal
优化 `sqlite_session.rs` 写路径，减少 `write_full_store` 全量序列化频率，优先使用增量写入。

## Output
- `crates/pony-agent-core/src/agent/sqlite_session.rs`

## Acceptance Criteria
1. Turn 热路径优先调用 `upsert_session`（增量）而非 `save_store`（全量）
2. 全量写仅在必要场景触发（session 删除、结构变更）
3. 编译通过，现有测试全部通过

## Current Progress
- 待开始

## Next Action
- 分析 `save_store` 调用链，识别可降级为增量写的路径
