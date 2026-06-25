# Design

## Blocking boundary inventory

- File IO, SQLite, serialization, and CPU-heavy transforms SHALL be explicitly classified.

## Helper layer

- The codebase SHALL provide a unified helper boundary for blocking work.
- Async tasks SHALL route blocking work through the helper layer rather than inline execution.
- The helper SHALL support both `spawn_blocking` and `block_in_place` strategies.
- The helper SHALL expose an `into_async()` transition interface that PA-068 can consume to replace turn-path blocking with async task orchestration.

## Transition contract with PA-068

- `tauri_adapter.rs` functions (`spawn_turn_stream`, `spawn_graph_run_stream`) SHALL retain their external signatures through PA-067; their internals MAY use the new blocking helper.
- These functions SHALL be explicitly annotated as "PA-068 transition targets" so that PA-068 knows exactly what to replace.
- PA-067 SHALL NOT fully rewrite the turn execution path — only the blocking execution mechanism.

## Cleanup

- After PA-067 and PA-068 are both complete, scattered `spawn_blocking` calls not routed through the unified helper SHALL be removed.
- `std::sync::Mutex` usage in `tauri_adapter.rs` and `lib.rs` SHALL be reviewed and, where appropriate, replaced with `tokio::sync::Mutex`.
