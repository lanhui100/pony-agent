# Tasks: Add History Node Management

## 1. Core Data Model

- [x] 1.1 Define `HistoryNode`, `HistoryBranch`, `HistoryCursorState`, and `WorkspaceStateRef`
- [x] 1.2 Add persistence format and loading strategy in the session/history layer
- [x] 1.3 Define host-agnostic capability flags for workspace rollback support

## 2. Retrieval and Runtime Read Surface

- [x] 2.1 Extend runtime/session query surfaces to accept `nodeId`
- [x] 2.2 Extend retrieved context construction so it can rebuild from a specified node
- [x] 2.3 Ensure run-state reconstruction works for latest and historical nodes

## 3. Control Plane

- [x] 3.1 Add checkout command with `transcript_only` and `transcript_and_workspace`
- [x] 3.2 Add restore-latest and branch-switch commands
- [x] 3.3 Add fork creation semantics when a new run starts from a historical non-head node
- [x] 3.4 Define response payloads that tell the UI whether rollback was fully applied or degraded

## 4. Tauri Frontend

- [x] 4.1 Extend runtime store to track visible node, branch head, active branch, and historical mode
- [x] 4.2 Add history panel and node action menu
- [x] 4.3 Add branch-aware restore/switch interaction affordances
- [x] 4.4 Prevent latest-only retrieval/runtime assumptions after checkout

## 5. Validation

- [x] 5.1 Add core tests for checkout, restore, fork, and branch switching
- [x] 5.2 Add retrieval/runtime tests for historical node reconstruction
- [x] 5.3 Add frontend store/component tests for historical mode and branch switching
- [x] 5.4 Document degraded behavior when workspace rollback is unavailable
