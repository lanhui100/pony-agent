# Proposal: Add History Node Management

## Why

Pony Agent core already supports graph runs, checkpoints, stop, and resume, but the product still behaves as a latest-only linear conversation. Users cannot safely move to an earlier historical point, inspect or restore prior context, branch from that point, or return to the previous branch head with a clear system model.

This gap now blocks a coherent implementation of:

- stop and continue across historical checkpoints
- transcript-only undo
- transcript-and-workspace undo
- restore-to-latest when no new action happened after checkout
- fork creation when a new action starts from a historical node
- return to the old branch head when a fork result is not accepted

Without a formal history graph, these behaviors would be implemented as ad hoc frontend state mutations and would drift across host surfaces.

## What Changes

This change introduces a first-class history node management model for Pony Agent:

- add immutable history nodes and explicit branches
- add cursor state to distinguish visible node, branch head, and workspace-applied node
- define checkout semantics for transcript-only and transcript-and-workspace restoration
- define restore-latest, fork, and branch switching behavior
- extend retrieval and runtime read models so they can reconstruct state from a specified history node rather than latest-only state
- define Tauri frontend interaction requirements for history browsing and branch-aware actions

## Scope

In scope:

- agent core history graph data model
- history-aware retrieval/runtime query surface
- host control-plane commands for checkout/restore/fork/switch
- Tauri UI interaction contract
- workspace rollback capability abstraction and graceful degradation

Out of scope:

- full visual graph editor
- cross-device history sync platform
- automatic git integration as a mandatory dependency
- archival/retention policy for old history nodes beyond basic persistence requirements

## Impact

- affects `runtime`, `graph`, `session`, `context`, `control_plane`, and Tauri frontend runtime state
- introduces a new cross-surface contract that must remain host-agnostic
- creates a stable base for future workflow mode, review/replay, and audit features
