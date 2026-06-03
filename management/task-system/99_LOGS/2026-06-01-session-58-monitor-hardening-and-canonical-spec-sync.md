# 2026-06-01 Session 58 - Monitor Hardening and Canonical Spec Sync

## Summary
- Spawned three sub-agents for parallel read-only review:
  - task system / OpenSpec drift check
  - PA-024 spec coverage audit
  - Model Monitor engineering-risk audit
- Applied two low-risk fixes to the delivered monitor surface:
  - prevent stale drill-down responses from overwriting the latest session selection
  - keep same-named models separated across providers in monitor model aggregation
- Synced canonical specs into `openspec/specs/` for:
  - `model-monitor-telemetry`
  - `history-node-management`
  - `cache-hit-optimization`

## Code / Docs
- [src/components/ModelMonitorPage.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/ModelMonitorPage.vue)
- [tests/ModelMonitorPage.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/ModelMonitorPage.spec.ts)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [management/task-system/01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [management/task-system/00_DASHBOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
- [openspec/specs/model-monitor-telemetry/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/model-monitor-telemetry/spec.md)
- [openspec/specs/history-node-management/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/history-node-management/spec.md)
- [openspec/specs/cache-hit-optimization/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/cache-hit-optimization/spec.md)

## Validation
- `cmd /c npm run test:unit -- tests/ModelMonitorPage.spec.ts`
- `cmd /c npm run test:unit -- tests/ModelMonitorPage.spec.ts tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/runtime-store.spec.ts`
- `cargo test model_dimension_key_keeps_same_model_names_separate_across_providers --manifest-path src-tauri/Cargo.toml`
- `cargo test load_model_monitor --manifest-path src-tauri/Cargo.toml`

## Remaining Workflow Gap
- Completed changes under `openspec/changes/` are not archived yet.
- Now that canonical specs exist, the next OpenSpec hygiene step is archive/closeout rather than spec sync.
