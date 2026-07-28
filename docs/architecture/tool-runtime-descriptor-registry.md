# PA-076: Tool Descriptor and Registry — Single Truth Source for Tool Metadata

## Motivation

Tool metadata had multiple truth sources derived from the tool **name** rather
than declared per tool. `builtin_tools()` returned 17 internal primitive
definitions; a separate set of `*_for_name()` lookup functions derived kind,
exposure, permission facts, and display metadata by matching on that name; and
`builtin_tool_contract_views()` deduplicated 17 primitives down to the 13
model-visible product names with its own ordering rules.

Two consequences mattered:

- Provider payload generation inferred "this is the builtin set" by comparing
  the tool array's **length and contents** against `builtin_tools()`, so any
  caller-supplied array of the same size took a different code path.
- Adding a tool meant touching several name-matching tables, and nothing
  enforced that they stayed consistent.

Phase 2 of PA-076 replaces name-derived lookup with a descriptor that carries
its own facts, and a versioned registry snapshot that validates them.

## Design

`ToolDescriptor` carries identity, schema, kind, exposure, typed permission
declaration, execution policy, display metadata, handler provenance, and source
revision. `ToolRegistrySnapshot` owns an ordered `Vec<ToolDescriptor>` plus an
alias index, and rejects invalid sets at construction.

```
builtin_tools()  ──►  ToolRegistrySnapshot::from_builtin_definitions()
(17 legacy defs)         │
                         │  per-primitive descriptor construction
                         │  + model-visible slot ordering
                         ▼
                   ToolRegistrySnapshot  ──►  TurnToolView  ──►  provider payload
                   (ordered descriptors)      (per-turn          capability registry
                                               exposure)          planner views
                                                                  list_available_tools
```

`ToolSurface` pairs a registry snapshot with its default `TurnToolView` and is
the single projection entry point for hosts.

### Registry position is the ordering truth source

The provider tool array order is part of the cache-friendly stable prefix
established by PA-025 / PA-029. Reordering it invalidates the prefix and costs
cache hits.

The legacy `builtin_tool_contract_views()` ordered by each product name's
**first appearance** among the 17 primitives. That detail is load-bearing:
`time_now` appears before `workspace_run_command` and both project to the
product name `Run`, so `Run` occupied slot 1 even though
`workspace_run_command` is the primitive that wins exposure.

Rather than replicate that rule at each projection point, `from_builtin_definitions`
sorts descriptors once at construction: model-visible descriptors keep their
product name's first-appearance slot, and internal/deferred descriptors follow
in stable definition order. Every downstream projection then just filters in
registry order.

**Invariant:** registry descriptor order *is* provider tool array order. A
projection must not re-sort.

### Unknown names pass through unchanged

`model_visible_tool_name` had a `_ => "Run"` fallback, so any unrecognized tool
name — including external MCP and dynamic tools — was silently relabeled as a
builtin product name. This was a second name-inference path alongside the
tool-count inference that task 2.7 removed.

Three functions now separate the cases:

| Function | Unknown name behavior | Use |
|---|---|---|
| `model_visible_tool_name_opt` | `None` | Callers that must distinguish |
| `product_visible_tool_name` | passes through unchanged | Contract views |
| `model_visible_tool_name` | `"Run"` (legacy) | Known primitives needing `&'static str` |

`ToolDefinition`, `ToolCall`, and `ToolResult` contract views use
`product_visible_tool_name`.

**Invariant:** no external or unknown tool name is ever labeled as a builtin
product tool.

## Validation Rules

`ToolRegistrySnapshot::from_descriptors` rejects:

- empty snapshot id, empty identity fields, incomplete source provenance
- duplicate descriptor ids
- ambiguous aliases (one alias resolving to two descriptors)
- descriptor id not matching its declared source namespace
  (`builtin:` / `mcp:` / `skill:` / `dynamic:`)
- source-provenance mismatch, including a non-builtin descriptor claiming
  `builtin-tools` provenance or an MCP/skill descriptor whose id does not
  belong to its provenance source
- external descriptors colliding with reserved builtin aliases
- composite dependencies referencing unknown children, and dependency cycles

`replace_source` swaps one source's descriptors atomically and refuses to let an
external source replace builtin descriptors.

## Ports Defined but Not Yet Wired

`tool_runtime.rs` holds contract-only boundaries for later phases. They compile
and have tests, but no production path consumes them yet:

| Type | Phase |
|---|---|
| `ToolDispatcher`, `InvocationOrigin`, `PrimitiveToolHandler` | 3 |
| `PendingControlRequest` (session/run/turn/call binding, digests, nonce, version, expiry) | 4 |
| `SandboxBackend`, `ProcessBackend` | 5 |
| `WebResolver` | 6 |
| `McpTransport` | 7 |
| `FakeClock`, `FakeResolver`, `FakeMcpTransport` | test harness |

`ToolOutcome` splits `execution_status` (`Ok` / `Error` / `Cancelled`) from an
orthogonal `control_outcome`, with `from_legacy_result` / `into_legacy_result`
adapting the legacy `ToolResult` string envelope. A pending control request has
no provider-consumable result, so `into_legacy_result` fails closed with
`control_outcome_pending` rather than fabricating one.

`Ask`, `Run`, and arbitrary-URL `WebFetch` remain fail-closed until their
phases land.

## Code Layout

| File | Change |
|---|---|
| `crates/pony-agent-core/src/agent/tools.rs` | `ToolDescriptor`, `ToolRegistrySnapshot`, `ToolSurface`, typed `ToolPermissionDeclaration`, `ToolOutcome`, slot ordering, name pass-through |
| `crates/pony-agent-core/src/agent/tool_runtime.rs` | Dispatcher / control-request / sandbox / process / resolver / MCP ports and fake harnesses |
| `crates/pony-agent-core/src/agent/provider.rs` | Payload generation reads registry projection; tool-count inference removed |
| `crates/pony-agent-core/src/agent/capability_bridge.rs` | Builtin capability permissions read descriptor facts |
| `crates/pony-agent-core/src/agent/planner.rs` | Planner tool views read `ToolSurface` |
| `src-tauri/src/lib.rs` | `list_available_tools` reads `ToolSurface` |

## Building and Testing on Windows

MSVC environment variables are not loaded by default. Use the wrapper:

```bash
cmd //c "scripts\run-rust-msvc.bat cargo test -p pony-agent-core --lib --target-dir target-test-exact-a agent::tools::"
```

`scripts/invoke-rust-target.ps1` (behind `npm run cargo:test:*`) pins
`--manifest-path src-tauri/Cargo.toml`, so `--lib` there targets the Tauri
crate. Core crate tests need an explicit `-p pony-agent-core`.

Phase-2 gate: `agent::tools::`, `agent::tool_runtime::`, and
`--test tool_router_regression`.
