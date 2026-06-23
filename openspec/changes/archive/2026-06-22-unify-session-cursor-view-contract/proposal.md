# Proposal: Unify Checkpoint / Cursor / View Contract for Multi-Surface Hosts

## Why

After the initial history node management (PA-028 / history-node-management canonical spec), the system has `checkpoint graph`, `history cursor`, and `runtime view` working but with two structural risks:

1. Frontend local cache and backend authoritative cursor both participate in recovery semantics, creating divergent restoration behavior between in-progress and loaded sessions
2. The responsibilities of `checkpoint graph`, `cursor`, and `view` are not yet tightened into distinct layers, making multi-surface (Tauri / TUI / CLI / HTTP) extension fragile

This gap became visible when a session could lose its historical rollback state after switching away and back, while a historical loaded session correctly preserved it.

## What Changes

This change introduces a three-layer contract:

1. **HistoryGraph** — immutable history nodes and explicit branches; the sole authority for branch topology and branch head state
2. **Cursor** — the single authoritative position: current visible node, active branch, view mode; NOT a second authority for branch latest
3. **View** — host-projected read model derived from graph + cursor; clients consume this rather than reconstructing history state locally

Implementations:

- Add authority mode (`host_authoritative` / `local_preview`) to runtime view and cursor state types
- Add resolved visible node, active branch head, and `isAtBranchHead` projection fields to `SessionRuntimeView`
- Add real `cursor_version` field to `HistoryCursor` with stale mutation conflict detection
- Disable branch/fork/restore host-level history control in browser preview degraded mode
- Remove frontend local cursor fallback from host-backed session loading path

## Scope

In scope:

- Rust types: `HistoryCursorState.cursor_version`, `SessionRuntimeView.authority_mode/resolved_visible_node_id/active_branch_head_node_id/is_at_branch_head`
- Frontend store: consume host-projected read-model fields; pass `expectedCursorVersion` to history-control Tauri commands
- Conflict detection: reject stale cursor mutations with explicit error message
- Browser preview: disable `restoreBranchHead / forkHistoryNode / switchHistoryBranch`

Out of scope:

- Full real `cursor_version` concurrency implementation (placeholder for now, actual atomic CAS deferred)
- HTTP / CLI adapter surfaces (contract defined, wiring deferred)
- Frontend visual history-tree editor

## Impact

- Affects `session.rs`, `runtime/mod.rs`, `control_plane.rs`, `lib.rs`, `runtime.ts`, `runtime.history.v1` persistence
- Adds backward-compatible fields to existing structs (all `Option` / `#[serde(default)]`)
- Browser preview users lose fake branch/fork/restore capabilities (explicit error instead)
- Existing saved sessions gain `cursor_version` field on next cursor mutation
