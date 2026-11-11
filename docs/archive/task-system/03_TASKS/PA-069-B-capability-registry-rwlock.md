# PA-069-B capability_registry Mutex → RwLock

## Basic Info
- ID: PA-069-B
- Status: Done
- Priority: P2
- Owner: @agent
- Created At: 2026-06-25
- Updated At: 2026-08-08（实现验证通过并收口）
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
- 已完成并收口（2026-08-08）。
- 代码验证：`RwLock<CapabilityRegistry>` 落地于 `control_plane/mod.rs:846`（HostControlPlane 字段）与 `runtime/turn_runner.rs:24`（TurnContext 字段，`Arc<RwLock<CapabilityRegistry>>`）。
- TOCTOU 修复：`apply_skill_source_snapshot` 采用「读 → 释放 → 应用 → 写回重验证」模式（见 `docs/concurrency/lock-ordering.md` 规则 3）。

## Next Action
- 无。已完成收口。
