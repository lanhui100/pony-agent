# Tasks

- [ ] Implement minimal per-session async turn task spike
- [ ] Replace blocking turn spawn path with async task orchestration (`tauri::async_runtime::spawn`)
- [ ] Rewire cancellation / resume / terminal cleanup in async model
- [ ] Validate multi-session concurrent execution behavior
- [ ] Remove deprecated `spawn_blocking` turn paths from `tauri_adapter.rs`
- [ ] Update integration tests (`runtime-store.spec.ts` etc.) for async model
- [ ] Final audit: confirm zero `spawn_blocking` in turn execution paths
