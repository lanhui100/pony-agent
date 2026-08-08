# PA-069-C Mutex 中毒容错修复

## Basic Info
- ID: PA-069-C
- Status: Done
- Priority: P2
- Owner: @agent
- Created At: 2026-06-25
- Updated At: 2026-08-08（实现验证通过并收口）
- Estimated Effort: 1.5h

## Goal
将项目关键路径上的 `lock().expect("poisoned")` 替换为 poison-tolerant 模式，防止 Mutex 中毒后连锁崩溃。

## Output
- `crates/pony-agent-core/src/agent/control_plane.rs`
- `crates/pony-agent-core/src/agent/runtime/mod.rs`
- 其他含 `.expect("poisoned")` 的文件

## Acceptance Criteria
1. 关键 `Mutex`/`RwLock` 使用 `lock().unwrap_or_else(|e| e.into_inner())` 或等效容错
2. 非关键路径可保留 `.expect()` 但添加注释说明
3. 编译通过，测试通过

## Current Progress
- 已完成并收口（2026-08-08）。
- 分类策略落地：`control_plane`（graph_runs / terminal / capability_registry / sessions_rwlock）、`execution_control.rs`（state）、`runtime` 各子模块（sessions / store）共 85+ 处使用 `unwrap_or_else(|e| { eprintln!(...); e.into_inner() })` 容错。
- `runtime` 锁按策略保留 `.expect("runtime lock poisoned")`（10 处，`control_plane` 各子模块）——复杂状态机中毒后继续更危险，属有意之举。
- 其余 `.expect("...poisoned")` 残留位于 dispatcher / plan_state / process / turn_flow 等 PA-069 范围外模块，未纳入本卡。

## Next Action
- 无。已完成收口。
