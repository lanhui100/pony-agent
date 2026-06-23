# Design: Session Cursor View Contract

## Overview

Build on the existing `history-node-management` contract to add a clean three-layer separation: immutable HistoryGraph, single authoritative Cursor, and derived View. Clients issue host commands and consume host-projected views instead of reconstructing history state locally.

## Key Decisions

### 1. Authority Mode

Every `SessionRuntimeView` and `HistoryCursorState` carries an `authority_mode` field:

- `host_authoritative` — returned by formal host-backed session store (Tauri / Rust backend)
- `local_preview` — returned by browser preview / local storage fallback

Clients use this field to decide which UI actions to enable (restore / fork / branch switching are disabled in `local_preview` mode).

### 2. Host-Projected Read Model

The `SessionRuntimeView` struct now carries top-level projection fields so clients do not need to infer history mode from raw cursor fields:

- `resolved_visible_node_id` — the node being viewed (may differ from `historyCursor.visibleNodeId` when cursor is null)
- `active_branch_head_node_id` — the current branch's head node
- `is_at_branch_head` — whether the view is showing the branch head (live) or an earlier node (historical)
- `cursor_version` — real monotonic version number from the backend cursor, or null in preview mode

### 3. Real Cursor Versioning

The `HistoryCursor` struct gains a `cursor_version: u64` field that is bumped on every cursor mutation (checkout / restore / fork / switch). History-control commands accept an `expected_cursor_version: Option<u64>` parameter. On mismatch, the mutation is rejected with an explicit conflict error.

### 4. Browser Preview Degradation

Browser preview / local fallback mode is explicitly downgraded:

- `restore_branch_head()` returns null with `sessionError`
- `forkHistoryNode()` returns null with `sessionError`
- `switchHistoryBranch()` returns null with `sessionError`
- `checkoutHistoryNode()` continues to work (transcript-only operations for message rollback)
- Runtime view returned in preview mode carries `authority_mode: "local_preview"`

### 5. Frontend Store Hard Cut

The `loadSessionState()` method no longer:

- Uses persisted `visibleNodeId` as fallback nodeId for host-backed sessions
- Applies `previousHistoryState` compensation for host-backed sessions

Host-backed sessions now strictly trust the `SessionRuntimeView` from the backend.

## Layer Responsibilities

```
HistoryGraph ← branch topology / branch head authority
     |
    Cursor ← current visible position (versioned)
     |
    View ← projected read model (authority + resolved fields)
     |
   Client ← consumes view, issues commands
```

## Compatibility

All new Rust fields are:

- `#[serde(default)]` for backward-compatible deserialization
- `Option` types or have sensible defaults
- Existing TS types remain structurally compatible; new fields are optional in type definitions
