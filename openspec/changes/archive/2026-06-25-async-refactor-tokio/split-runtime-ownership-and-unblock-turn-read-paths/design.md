# Design

## Ownership split

- `SessionStore` SHALL be treated as a distinct ownership domain.
- `GraphRunStore` SHALL be treated as a distinct ownership domain.
- Provider/tool/planner execution state SHALL NOT remain implicitly bundled under one monolithic runtime lock.

## Lock and access model

- Read-plane commands (`list_sessions`, `load_session_runtime_view`, `load_retrieved_context`) SHALL have a path that does not require holding the same execution lock used by a full turn.
- Turn execution SHALL narrow lock scope to setup / commit boundaries rather than hold a global runtime lock for the full turn lifetime.

## Compatibility

- Existing event contracts SHALL remain valid while ownership is being split.

## Cleanup and migration

- Once the new ownership model reaches steady state, all code paths that bypass the new model to access the old `Mutex<AgentRuntime>` SHALL be removed.
- `control_plane.rs` SHALL no longer hold `Mutex<AgentRuntime>` as a single monolithic lock.
- `#[cfg(test)]` tests relying on the old lock model SHALL be migrated to the new ownership boundaries.
