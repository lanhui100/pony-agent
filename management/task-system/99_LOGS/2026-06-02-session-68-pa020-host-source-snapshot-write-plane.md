# 2026-06-02 Session 68 - PA-020 Host Source Snapshot Write Plane

## Summary
- Added a host-internal MCP source snapshot write plane for `PA-020`.
- Kept Tauri on capability read-plane only; no frontend registration command was introduced.
- Synchronized the same normalized MCP snapshot into both `HostControlPlane` and `AgentRuntime`.
- Extended tool-call resolution so host-registered MCP tool labels can enter the existing runtime bridge path.

## Decisions
- Production registration uses a normalized `McpSourceSnapshot` rather than raw MCP protocol payloads.
- Refresh semantics are source-scoped replacement, not append-only registration.
- A source with `degraded` or `unreachable` availability remains inspectable even if the latest capability list is empty.
- Builtin aliases keep precedence; MCP-backed tool resolution is the fallback path for non-builtin tool labels.

## Code
- `src-tauri/src/agent/capability_bridge.rs`
  - added `McpSourceSnapshot`
  - added source-scoped replacement in `CapabilityRegistry`
  - added MCP tool-label fallback in `resolve_tool_call()`
- `src-tauri/src/agent/control_plane.rs`
  - added `ApplyMcpSourceSnapshotCommand`
  - added host-internal snapshot validation and synchronized apply path
- `src-tauri/src/agent/runtime.rs`
  - added runtime snapshot apply helper for registry sync
- `openspec/changes/add-mcp-capability-bridge/design.md`
  - documented host-internal snapshot write surface and replacement semantics
- `openspec/changes/add-mcp-capability-bridge/specs/mcp-capability-bridge/spec.md`
  - added acceptance scenarios for source refresh and unavailable source visibility
- `openspec/changes/add-mcp-capability-bridge/tasks.md`
  - marked `5.3` and `6.2` done

## Validation
- `cargo test --manifest-path src-tauri/Cargo.toml apply_mcp_source_snapshot -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml capability_bridge_resolves_host_registered_mcp_tool_snapshot -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml registry_resolves_mcp_tool_resource_and_prompt_template_actions -- --nocapture`
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`

## Remaining
- No real MCP discovery/connector exists yet; snapshots are still host-injected.
- `resource` fetch and `prompt_template` expansion still stop at resolve contracts.
- observability and planner-consumption acceptance work remains open.
