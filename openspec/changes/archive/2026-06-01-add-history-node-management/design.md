# Design: History Node Management

## Overview

The system should stop treating session history as a mutable linear chat transcript and instead model it as an immutable history graph with explicit branches and a movable cursor.

The design is intentionally split into three layers:

1. event log
2. materialized history graph
3. current cursor state

This allows the core to support undo/restore/fork semantics without deleting prior state and without binding the implementation to a single host or workspace backend.

## Core Model

### History Node

Each stable historical point is represented by an immutable node.

Suggested fields:

- `nodeId`
- `sessionId`
- `parentNodeId`
- `branchId`
- `forkedFromNodeId`
- `kind`
- `transcriptRef`
- `runRef`
- `workspaceRef`
- `summary`
- `createdAtMs`

`kind` should at minimum distinguish:

- `turn_committed`
- `turn_cancelled`
- `run_paused`
- `checkpoint`
- `manual_snapshot`

### Branch

Branches must be explicit persistent objects.

Suggested fields:

- `branchId`
- `sessionId`
- `baseNodeId`
- `headNodeId`
- `forkedFromBranchId`
- `forkedFromNodeId`
- `label`
- `createdAtMs`
- `updatedAtMs`

### Cursor State

Cursor state distinguishes what the user is looking at from what is the latest state and what has been applied to the workspace.

Suggested fields:

- `sessionId`
- `visibleNodeId`
- `activeBranchId`
- `branchHeadNodeId`
- `workspaceNodeId`
- `mode`

`mode` should support:

- `live`
- `historical`
- `historical_dirty`

## Workspace State Abstraction

Core must not assume git, Tauri, or local desktop filesystem semantics.

Use an abstract workspace reference:

- `kind: none | git_commit | patch_set | host_snapshot`
- `locator`
- `rollbackCapable`

Behavior:

- if the host supports workspace rollback, `transcript_and_workspace` checkout may restore both transcript and workspace
- if the host does not support rollback, the same request degrades cleanly to transcript-only checkout with a capability flag in the response

## Query and Control Surfaces

### Read APIs

The retrieval/runtime surfaces should accept history context:

- `load_history_graph(sessionId)`
- `load_history_cursor(sessionId)`
- `load_session_runtime_view(sessionId, nodeId?, runId?)`
- `load_retrieved_context(sessionId, nodeId?, runId?)`

### Write/Control APIs

The control plane should expose:

- `checkout_history_node(sessionId, nodeId, mode)`
- `restore_branch_head(sessionId, branchId?)`
- `fork_from_history_node(sessionId, nodeId)`
- `switch_history_branch(sessionId, branchId)`

Starting a new action from a historical node should either:

- keep the existing branch if the visible node is already the head, or
- create a new branch if the visible node is older than the current branch head

## Behavioral Rules

### Undo is checkout, not deletion

The system never deletes future nodes when the user goes to an earlier node. It only moves the cursor and optionally the workspace-applied state.

### Restore latest only applies before divergence

If the user checked out an earlier node but has not created a new action, restore-latest returns to the original branch head.

If the user created a new action from the earlier node, a fork exists and restore behavior becomes branch-aware.

### Forks are first-class

A new action from a non-head historical node always creates a new branch record. Returning to the old branch is a normal branch switch, not a special undo exception.

## Tauri Interaction Model

The frontend should not attempt to infer branching semantics from raw messages.

Required UI surfaces:

1. history panel with node cards
2. node action menu
3. top status strip showing current time-position
4. branch-aware recovery actions

Minimum node actions:

- checkout transcript only
- checkout transcript and workspace
- continue from here
- switch to this branch head

## Migration Notes

The current session store is latest-oriented. First implementation may use snapshot-style refs to reduce risk, then optimize to incremental storage later.

This change should prefer correctness and explicit semantics before storage compaction.
