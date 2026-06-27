# PA-068: Per-Session Async Turn Task Model

## Motivation

Before this change, every turn execution was dispatched via
`spawn_blocking` — a dedicated OS thread blocked for the entire turn's
lifetime. Combined with the old global `Mutex<AgentRuntime>`, this meant
sessions effectively serialized: only one turn could run at a time, and
switching to a historical session's read view would block until the active
turn finished. With PA-065 (ownership split) and PA-066 (async provider)
complete, the technical prerequisites for genuine per-session concurrency
were in place.

## Design

### TurnTaskRegistry

A `TurnTaskRegistry` manages per-session async task identities:

```rust
pub struct TurnTaskRegistry {
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    max_concurrent: Option<usize>,
}
```

- **`register(session_id, handle)`** — stores the task handle keyed by
  session. If the session already has a running task, the old one is
  aborted (auto-cancellation of stale turns). A `max_concurrent` cap
  rejects new tasks when the limit is reached.
- **`unregister(session_id)`** — removes the handle (called by
  `TaskCleanupGuard::drop`).
- **`abort_all()`** — drains all handles and aborts every task; wired to
  Tauri window close.

### TaskCleanupGuard

A `Drop` guard that auto-unregisters from the registry when the async task
completes (normally or by abort):

```rust
struct TaskCleanupGuard { app: AppHandle, session_id: String }

impl Drop for TaskCleanupGuard {
    fn drop(&mut self) {
        self.app.state::<TurnTaskRegistry>().unregister(&self.session_id);
    }
}
```

### Execution Model

```
spawn_turn_stream(command)
  │
  ▼
TurnTaskRegistry.register(session_id, handle)
  │
  ▼
tauri::async_runtime::spawn(async {
    let _guard = TaskCleanupGuard::new(...);
    let sink = TauriTurnEventSink::new(...);
    spawn_blocking(move || {
        control_plane.start_turn_stream(&sink, command);
    }).await;
})
```

The outer `spawn` creates a true async task on the tokio runtime. Inside,
`spawn_blocking` is still used for the synchronous `start_turn_stream` call
(the `AgentRuntime` execution path remains sync). The key advance: each
session gets its own async task identity, enabling:
- **Per-session cancellation**: abort one session's task without affecting others
- **Concurrent sessions**: multiple sessions' turn tasks coexist on the
  tokio runtime; the RwLock on sessions allows concurrent reads across
  tasks
- **Clean shutdown**: `abort_all()` on window close kills all tasks cleanly

### Migration from PA-067

| Before (PA-067 state) | After (PA-068) |
|---|---|
| `spawn_blocking` directly | `TurnTaskRegistry` + `spawn` outer layer |
| No per-session identity | Per-session handle registration |
| Manual cleanup | `TaskCleanupGuard` auto-unregisters |
| No abort capability | `abort_all()` + per-session abort |

## Key Decisions

- **Outer `spawn` + inner `spawn_blocking`**: The two-layer approach avoids
  a full async rewrite of `AgentRuntime` while still giving each session an
  independent async task identity. A future PA could push async further into
  the runtime.
- **Session-id keyed registry**: Using `session_id` (not `turn_id`) means
  rapid re-submissions by the same session auto-cancel the previous turn, a
  natural UX expectation.
- **`TaskCleanupGuard` over manual try/finally**: Rust's Drop semantics
  guarantee cleanup runs even if the task is aborted midway, preventing
  registry leaks.

## Code Layout

| File | Change |
|---|---|
| `src-tauri/src/turn_task_registry.rs` | New file: `TurnTaskRegistry` with register/unregister/abort_all |
| `src-tauri/src/tauri_adapter.rs` | `spawn_turn_stream`/`spawn_graph_run_stream` switched to outer `spawn`; `TaskCleanupGuard` added |
| `src-tauri/src/blocking_helper.rs` | `TaskCleanupGuard` pre-positioned in PA-067, now wired to registry |
