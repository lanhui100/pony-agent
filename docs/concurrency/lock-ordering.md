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

### Workspace 域锁（PA-080，与 `sessions_rwlock` 同级、先 registry 后 authorize）

```
5a. workspace_registry（SessionStore.workspaces 内联字段，随 sessions_rwlock 保护）
5b. path_authorizations: RwLock<AuthorizeStore>   ← 授权清单（先 registry 后 authorize）
```

- `path_authorizations` 是 `SessionStore` 内的 `Arc<RwLock<AuthorizeStore>>`（`session.rs`），与 `sessions_rwlock` 同级：先取 registry（workspaces 字段）再取 authorize。
- **判定闭包内不得再获取 `sessions_rwlock` 写锁**：`classify_path` 的授权查询只读 `path_authorizations`，工具执行路径不得在持有授权锁时反向获取 sessions 写锁（避免与 `sessions_rwlock` 形成反向依赖）。
- 授权清单每次变更即写 `store_metadata` key=`path_authorizations.v1`（SQLite）/ JSON fallback。

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

5. **禁止在持有 `graph_runs` 时调用会再次加锁的方法（非重入 Mutex 作用域纪律，ADR 0032）。**
   `graph_runs` 是 `std::sync::Mutex`（非重入），下列方法内部会再次 `self.graph_runs.lock()`：
   `resolve_graph_run_for_retrieval` / `load_graph_run` / `load_execution_checkpoint` /
   `load_active_graph_run_for_session`（另有 18 个自动发现的 locker）。在 guard 作用域内调用它们会造成
   **同线程自死锁**——2026-10-14 的桌面端「进入会话页卡死」P0 即由此产生
   （`resolve_graph_run_submission_plan` 的 `.or_else` 落在 guard 内）。
   - 命中路径（`run_id` 有值且命中）必须**单 guard 原子完成**「快照 + ask-wait 判定」；
   - fallback 路径必须**先释放锁**再解析，随后用独立短锁重取所需数据。
   - 机械门禁：`node scripts/check-graph-run-lock-scope.mjs`（禁用集合 = 基线 4 名 ∪ 自动发现全部 locker；
     发现数退化即 Exit 2；`--self-test` 含违规/合法/注释/顺序加锁样例）与
     `python3 .dev-team/check-lock-discipline.py`（独立实现交叉校验）。

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
| `path_authorizations` | `RwLock` | `AuthorizeStore`（`HashMap<canonical_path, AuthorizedPathEntry>`，path_permission.rs；随 sessions_rwlock 同级，先 registry 后 authorize） | Recover — `unwrap_or_else` |

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
