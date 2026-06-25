# Lock Ordering Specification

## Canonical Lock Order

All locks in `HostControlPlane` and `AgentRuntime` must be acquired in this order:

```
1. runtime: Mutex<AgentRuntime>            ← highest (state machine)
2. capability_registry: RwLock<Registry>   ← read-mostly
3. graph_runs: Mutex<GraphRunStore>        ← run lifecycle
4. frontend_diagnostics: Mutex<Connection> ← SQLite (diagnostics)
5. sessions_rwlock: RwLock<SessionStore>   ← lowest (backed by SQLite)
```

## Rationale

- `runtime` is the most complex state machine. If poisoned, the system cannot safely continue so it panics (`.expect()`). All other locks can tolerate poison.
- `capability_registry` is read-mostly with rare writes (MCP/skill source changes). Read locks never block each other.
- `graph_runs` manages run lifecycle transitions. Short-lived lock holds.
- `sessions_rwlock` is the most frequently accessed and backed by SQLite for crash recovery.

## Rules

1. **Never hold a lower lock when acquiring a higher lock.** E.g., do not hold `sessions_rwlock` while calling `runtime.lock()`.

2. **If both `sessions_rwlock` and `runtime` must be held**, always acquire `runtime` first, then `sessions_rwlock`.

3. **Drop guards before re-acquiring at a different level.** The `apply_skill_source_snapshot` pattern demonstrates this correctly:
   - Acquire `capability_registry.read()` → normalize → drop guard
   - Acquire `runtime.lock()` → apply → drop guard
   - Acquire `capability_registry.write()` → re-normalize + replace → drop guard

4. **Tolerate poison on all locks except `runtime`.** All other locks use `unwrap_or_else(|e| { eprintln!(...); e.into_inner() })`.

### Known Exception

`load_session_runtime_view` (`control_plane.rs:2831-2839`) acquires `sessions_rwlock.write()` before `runtime.lock()`. This is safe because:
- The `sessions_rwlock` guard is temporary and dropped before `runtime.lock()`
- There is no overlapping hold of both locks simultaneously
- However, this violates the canonical order and should be refactored if possible

## Lock Inventory

### HostControlPlane structural locks (`control_plane.rs:767-776`)

| Lock | Type | Fields Protected | Poison Strategy |
|------|------|------------------|-----------------|
| `runtime` | `Mutex` | `AgentRuntime` (graph, sessions handle, hooks, tools, planner, context, telemetry) | **Panic** — `.expect()` |
| `capability_registry` | `RwLock` | `CapabilityRegistry` (capability/skill index, cloned from `AgentRuntime`) | Recover — `unwrap_or_else` |
| `graph_runs` | `Mutex` | `GraphRunStore` (run metadata, lifecycle state) | Recover — `unwrap_or_else` |
| `frontend_diagnostics` | `FrontendDiagnosticsStore` (contains `Mutex<Option<Connection>>`) | SQLite diagnostics connection | Recover — `unwrap_or_else` (frontend_diagnostics.rs) |

### ExecutionControlRegistry internal locks (`execution_control.rs`)

| Lock | Type | Fields Protected | Poison Strategy |
|------|------|------------------|-----------------|
| `state` | `Mutex` | `HashMap<String, ExecutionCheckpoint>` | Recover — `unwrap_or_rule` |

### AgentRuntime fields (`runtime/mod.rs:282-293`)

| Lock | Type | Fields Protected | Notes |
|------|------|------------------|-------|
| `sessions` | `Arc<RwLock<SessionStore>>` | Session in-memory + SQLite backend | Shared with `HostControlPlane.sessions_rwlock` |

### Temporary/contextual locks

| Lock | Type | Location | Notes |
|------|------|----------|-------|
| `terminal` (RecordingTurnEventSink) | `Mutex` | `control_plane.rs`: inside `execute_graph_run_stream` closure | Not a structural lock; created per-stream |

## Lock Acquisition Verification

All lock acquisition patterns in `control_plane.rs` were verified. No instance was found where a lower lock is held while acquiring a higher lock, with the single exception documented above.
