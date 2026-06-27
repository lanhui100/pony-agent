# PA-065: Runtime Ownership Split — Session Read/Write Decoupling

## Motivation

`HostControlPlane` held `self.runtime: Mutex<AgentRuntime>`, which meant
every session read operation (`list_sessions`, `load_session_snapshot`,
`inspect_retrieved_context`, etc.) contended for the **same** lock that a
running turn held for its entire duration. Even with frontend caching, any
read-plane query that arrived during a turn would block until the turn
completed. This prevented genuine multi-session concurrency and made read
operations dependent on turn execution latency.

## Design

The `AgentRuntime.sessions` field was changed from `SessionStore` to
`Arc<RwLock<SessionStore>>`, and `AgentRuntime::sessions_handle()` was added
to share the `Arc` with `HostControlPlane`. `HostControlPlane` gained a new
field `sessions_rwlock: Arc<RwLock<SessionStore>>` that the builder extracts
from the runtime at construction time.

**Read-plane methods** in `HostControlPlane` (16 total) were migrated from
`self.runtime.read()` to `self.sessions_rwlock.read()`. Methods with lazy
initialization side effects (e.g. `snapshot_at`) use `write()` instead of
`read()` on the sessions lock. The `AgentRuntime` rwlock is only held for
turn execution (`run_turn`, `start_turn_stream`, `execute_graph_run_stream`)
and capability/planner mutations that require the full runtime state.

## Boundaries

```
┌────────────────────────────┐     ┌─────────────────────────┐
│   HostControlPlane         │     │   HostControlPlane      │
│   self.runtime (RwLock)    │     │   self.sessions_rwlock  │
│   ────────────────────     │     │   (Arc<RwLock>)         │
│   turn execution           │     │   list_sessions()       │
│   start_turn_stream()      │     │   load_session_         │
│   execute_graph_run_       │     │   snapshot()            │
│   stream()                 │     │   inspect_retrieved_    │
│   capability/planner       │     │   context()             │
│   mutations                │     │   load_history_graph()  │
└────────────────────────────┘     └─────────────────────────┘
```

## Key Decisions

- **`Arc<RwLock<SessionStore>>` over `Arc<Mutex<...>>`**: RwLock allows
  multiple concurrent readers (session list, inspection queries) while a
  single writer commits trace records or mutates session state.
- **`+ Sync` on `SessionBackend`**: Required because `RwLock<SessionStore>`
  must be `Send + Sync` when wrapped in `Arc`; the SQLite backend was
  already thread-safe.
- **No full runtime clone**: Sharing only the session store (not the whole
  `AgentRuntime`) avoids cloning provider resolvers, tool executors, hook
  registries, and capability registries — keeping the ownership model clean.

## Code Layout

| File | Change |
|---|---|
| `crates/pony-agent-core/src/agent/runtime/mod.rs` | `sessions: SessionStore` → `sessions: Arc<RwLock<SessionStore>>`; added `sessions_handle()` |
| `crates/pony-agent-core/src/agent/control_plane.rs` | Added `sessions_rwlock` field; migrated 16 read-plane methods off `self.runtime` |
| `crates/pony-agent-core/src/agent/session.rs` | Added `+ Sync` bound on `SessionBackend` trait |
