# Tasks

- [ ] Draw current ownership map for `AgentRuntime` / `HostControlPlane`
- [ ] Mark session-local, run-local, and global state
- [ ] Refactor read-plane access paths to avoid global execution lock coupling
- [ ] Narrow turn execution critical sections
- [ ] Add regression coverage for read-plane while turn execution is active
- [ ] Update `#[cfg(test)]` tests that depend on old global lock model
- [ ] Remove deprecated `Mutex<AgentRuntime>` patterns and dead code
