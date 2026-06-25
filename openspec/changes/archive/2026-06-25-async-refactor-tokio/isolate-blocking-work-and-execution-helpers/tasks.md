# Tasks

- [ ] Inventory blocking / CPU-heavy paths across `tauri_adapter.rs`, `session.rs`, `frontend_diagnostics.rs`, `lib.rs`
- [ ] Introduce shared blocking execution helpers with `spawn_blocking` / `block_in_place` support
- [ ] Provide `into_async()` transition interface for PA-068 takeover
- [ ] Migrate hot paths onto helper boundary (non-turn paths first)
- [ ] Mark `tauri_adapter.rs` turn paths as PA-068 transition points (do not fully rewrite)
- [ ] Verify helper usage in tauri adapter and runtime support code
- [ ] Update `lib.rs` `#[cfg(test)]` tests that depend on old blocking patterns
- [ ] Remove migrated scattered `spawn_blocking` calls (excluding PA-068-routed paths)
