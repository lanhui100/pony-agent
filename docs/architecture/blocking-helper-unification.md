# PA-067: Blocking Helper Unification

## Motivation

Even after provider calls became async (PA-066), the runtime still contained
scattered synchronous blocking work: file I/O, SQLite access, serialization,
and CPU-heavy computation. These were done via ad-hoc
`tauri::async_runtime::spawn_blocking` calls throughout the codebase,
making it unclear which operations were blocking, hard to audit, and
impossible to apply uniform policy (timeouts, thread pool limits,
monitoring).

## Design

A single `BlockingHelper` struct was introduced in `src-tauri/src/`:

```rust
pub struct BlockingHelper;

impl BlockingHelper {
    pub async fn spawn<T: Send + 'static>(
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, String> {
        tauri::async_runtime::spawn_blocking(f)
            .await
            .map_err(|e| format!("blocking helper spawn error: {e}"))
    }
}
```

All blocking work must go through this single entry point rather than
calling `spawn_blocking` directly. This gives a single place to add:
- Timeout wrapping
- Thread pool saturation monitoring
- Structured logging of blocking durations

### Migration Scope

Six frontend diagnostic Tauri commands (trace append, query, stall snapshot,
clear, JSON export, Chrome trace export) were migrated from direct
`spawn_blocking` to `BlockingHelper::spawn`.

## Key Decisions

- **Location in `src-tauri/` not `pony-agent-core/`**: Blocking work that is
  specific to the Tauri integration layer (frontend diagnostics, event
  dispatch) belongs in the Tauri crate. Core runtime blocking is handled
  separately via `runtime_helper::block_on`.
- **`TaskCleanupGuard` pre-positioned**: Although `TaskCleanupGuard` belongs
  to PA-068, it was introduced here with PA-068 transition markers in
  `tauri_adapter.rs` so the per-session task model could land without
  re-touching the same files.
- **No behavioral change**: Each migrated call behaves identically — the
  helper is a thin wrapper, not a semantic transformation.

## Code Layout

| File | Change |
|---|---|
| `src-tauri/src/blocking_helper.rs` | New file: `BlockingHelper::spawn` unified entry point |
| `src-tauri/src/tauri_adapter.rs` | Added PA-068 transition markers; `TaskCleanupGuard` skeleton |
| `src-tauri/src/` (frontend diag commands) | 6 Tauri commands migrated from raw `spawn_blocking` to `BlockingHelper::spawn` |
