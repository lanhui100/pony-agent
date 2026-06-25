# PA-069-B capability_registry Mutex → RwLock

## Basic Info
- ID: PA-069-B
- Status: Ready
- Priority: P2
- Owner: @agent
- Created At: 2026-06-25
- Estimated Effort: 1h

## Goal
将 `control_plane.rs` 中的 `capability_registry: Mutex<CapabilityRegistry>` 改为 `RwLock<CapabilityRegistry>`，消除读操作互斥。

## Output
- `crates/pony-agent-core/src/agent/control_plane.rs`

## Acceptance Criteria
1. `Mutex` 替换为 `RwLock`
2. 读路径使用 `read().expect(...)`，写路径使用 `write().expect(...)`
3. 编译通过，现有测试全部通过

## Current Progress
- 待开始

## Next Action
- 修改 `control_plane.rs` 中 `capability_registry` 类型
