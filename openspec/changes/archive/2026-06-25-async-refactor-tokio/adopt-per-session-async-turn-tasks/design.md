# Design

## Task model

- Each session/run SHALL be able to own an independent async turn task.
- Tauri SHALL use `tauri::async_runtime::spawn` for turn/run orchestration instead of blocking execution wrappers.

## Event model

- Existing turn event contracts SHALL remain the primary UI synchronization mechanism.
- Task completion, cancellation, and terminal cleanup SHALL preserve current frontend expectations.

## Preconditions

- Ownership split, async provider IO, and blocking helper boundaries MUST be in place before final integration.

## Cleanup

- Once per-session async task model is stable, `tauri_adapter.rs` SHALL remove `spawn_turn_stream` and `spawn_graph_run_stream` blocking variants entirely.
- `tauri_adapter.rs` SHALL have zero `spawn_blocking` calls in turn execution paths (frontend diagnostics `spawn_blocking` is managed by PA-067's unified helper).
- Integration tests (`runtime-store.spec.ts` etc.) SHALL be updated to validate async concurrent turn behavior.
