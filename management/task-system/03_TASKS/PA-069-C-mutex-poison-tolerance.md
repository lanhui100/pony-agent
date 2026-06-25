# PA-069-C Mutex 中毒容错修复

## Basic Info
- ID: PA-069-C
- Status: Ready
- Priority: P2
- Owner: @agent
- Created At: 2026-06-25
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
- 待开始

## Next Action
- grep 全项目查找 `.expect(".*poisoned")` 调用点
