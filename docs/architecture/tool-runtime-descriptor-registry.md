# PA-076: Tool Descriptor, Registry, and Governed Dispatcher — Single Truth Source for Tool Metadata

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

PA-076 replaces name-derived lookup with a descriptor that carries its own
facts, a versioned registry snapshot that validates them, and a governed
dispatcher that is the **single execution entry** for every top-level and
composite child invocation. Phases 1–7 and the runtime default switch are now
implemented; this document describes the resulting architecture and the two
remaining integrator notes.

## Design: Descriptor and Registry

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

## Governed Dispatcher — Single Execution Entry

`GovernedDispatcher` (in `dispatcher.rs`) is the single governed execution
entry. Every top-level and composite child invocation walks the same fixed
pipeline, in order:

1. **Origin authorization + exposure** — the invocation is bound to an
   unspoofable `InvocationOrigin`; a model origin can only call descriptors
   visible in the current `TurnToolView`, and internal descriptors are
   reachable only through a bounded `ChildDispatch`.
2. **Raw schema validation** of the incoming arguments.
3. **Mutable pre-dispatch hook** — hooks cannot rewrite descriptor identity or
   origin.
4. **Final validation/normalization + permission decision** — after the hook,
   the final identity and arguments are re-validated, re-authorized, and run
   through the policy evaluator (final-argument policy and sandbox decision).
5. **Control request binding** — allow/deny/approval/pending facts are bound to
   `descriptor_snapshot_id + final_args_digest + policy_digest +
   workspace/session/run/turn/call`, so security-relevant inputs cannot be
   rewritten after this point.
6. **Atomic budget reservation** — cancellation, deadline, concurrency, calls,
   bytes, and output are reserved from a shared atomic ledger.
7. **Handler execution** — the handler returns a domain result; it cannot
   fabricate permission evidence, control outcomes, or telemetry.
8. **Result normalization + exactly-once lifecycle record** — the dispatcher
   normalizes the result, settles budgets, emits telemetry, and produces one
   lifecycle record per top-level and child invocation (the runtime never
   re-emits the same layer's hooks).

Key supporting pieces:

- `PendingControlRequest` (CAS): a persisted request family for both `Ask` and
  approvals, distinguished by `request_kind`. It binds session/run/turn/call,
  descriptor snapshot id, final-args digest, policy digest, expiry, and a
  one-time nonce/version; answer/approve/cancel use compare-and-swap and fail
  closed on replay, expiry, cross-session, parameter change, or source revision
  change.
- Bounded child dispatch (`child_dispatch.rs`): composite handlers receive only
  a `ChildDispatch` carrying parent lineage, an allowed-child authority set, a
  shared atomically reserved `BudgetLedger`, a cancellation token, and the
  remaining deadline. Depth is capped (default 4), cycles/self-recursion are
  rejected, and once any child enters a pending approval/interaction state,
  unstarted siblings are stopped.
- Governed composites (`dispatcher_composites.rs`): `workspace_batch` and
  `workspace_gather_context` now run every child through the governed child
  dispatcher instead of calling legacy `execute_internal` directly, so each
  child independently resolves, validates, authorizes, reserves budget, and
  emits its own lifecycle evidence.
- Atomic budget ledger (`budget.rs`): one `BudgetLedger` per top-level
  invocation, shared via `Arc`; all reservations are atomic and fail closed
  (`reserve_call`, `begin_execution`, `reserve_bytes`, `check_deadline`).
- Permission aggregation: `aggregate_composite_permission` computes the
  conservative scope union and strongest approval/host-mediation requirement
  while preserving per-child evidence.

The matrix test suite (`crates/pony-agent-core/tests/dispatcher_matrix.rs`, 27
tests) covers builtin/MCP/skill/composite origins, model/child/host origins,
allow/deny/approval/control-suspend, hook rewrite re-authorization,
cancellation, atomic budget exhaustion, and partial results. The independent
phase-3 review is `management/task-system/02_REVIEWS/2026-08-02-pa076-phase3-review.md`.

## Runtime Default Switch (governed executor)

`AgentRuntimeBuilder` now defaults its tool executor to
`build_governed_executor(workspace_root)` from `governed_executor.rs`, so a
plain runtime executes every builtin primitive through `GovernedDispatcher`
instead of a bare `ToolRouter`. An explicit `tool_executor(...)` override still
wins.

- `build_governed_executor` bridges every builtin primitive to its legacy
  `ToolRouter` implementation through `RouterPrimitiveHandler`, registers the
  governed `workspace_batch`/`workspace_gather_context` composites with the
  same read-only child authority as the legacy migration gate, and registers a
  `LegacyCompatiblePolicyEvaluator` that reproduces legacy direct-execution
  behavior (every builtin allowed).
- Fidelity tests prove List/Search/Glob/Ask/Write/Edit outputs are byte-identical
  to legacy `ToolRouter`; `Run` correctly fails closed with
  `sandbox_unavailable` when no `SandboxBackend` is registered.
- `tool_router_regression` (13 tests) stays on the legacy `ToolRouter` path on
  purpose: it is the characterization gate proving the legacy surface did not
  change.

## Phase 4: Plan and Ask Control Tools

- `plan_state.rs`: session-owned, revisioned `Plan` control state. `PlanStore`
  holds plans with stable `plan_id`, `revision`, kind/summary, ordered steps
  (stable `step_id`), and a `Draft / Executing / Completed / Aborted` lifecycle.
  `PlanControlHandler` exposes create/replace/merge/complete-step mutations,
  all compare-and-swap on the revision. `Plan` is a state control — it never
  accepts a generic `calls` array or executes arbitrary child calls.
- `ask_control.rs`: host-facing `Ask` (`Interaction`) control adapter over a
  `GovernedDispatcher`. It shares the `PendingControlRequest` lifecycle with
  approvals but is distinguished by `PendingControlRequestKind::Interaction`;
  answering an Ask never equals approving a tool. Functions:
  `list_pending_asks`, `answer_ask`, `cancel_ask`, `expire_asks`, plus
  `ControlRequestAuthorization::for_request`.
- Graph Ask wait/resume: `GraphRunStore` persists `ask_waits` bound per run
  (`GraphAskWaitBinding`) and the graph refuses to finalize a run while
  unresolved waits remain. `bind_ask_wait` / `list_ask_waits` /
  `resume_ask_wait` suspend the run in `waiting_user` and inject exactly one
  terminal tool result for the original call id.
- Control-plane surface: `control_plane/ask_plan_commands.rs` exposes
  `list_pending_asks`, `answer_ask`, `cancel_ask`, `expire_asks`,
  `plan_create`, `plan_replace`, `plan_merge`, `plan_complete_step`,
  `plan_list`, `plan_get`, `graph_list_ask_waits`, `graph_bind_ask_wait`, and
  `graph_resume_ask` — 13 Tauri commands in `src-tauri/src/lib.rs`.
- Frontend: `src/stores/ask.ts`, `src/stores/plan.ts`,
  `src/components/AskPanel.vue`, `src/components/PlanPanel.vue`, and the shared
  contract types in `src/types/ask-plan.ts` (with vitest suites
  `tests/ask-store.spec.ts`, `tests/plan-store.spec.ts`,
  `tests/AskPlanPanel.spec.ts`).

## Phase 5: Process Lifecycle and Sandbox

- `process.rs`: `ProcessManager` is the production `ProcessBackend`. It starts
  processes and returns an opaque, high-entropy handle bound to the requesting
  session; every later operation (`poll`, `write_stdin`, `kill`) must present
  the same session id, cross-session use is rejected, and stale handles fail
  closed. stdout/stderr are drained concurrently into bounded ring buffers with
  truncation/dropped-byte evidence (`drain_stats`), and `kill_after` / `shutdown`
  cover timeout and session-shutdown cleanup.
- `sandbox.rs`: `SandboxSupportMatrix` records which containment strategy a
  platform can offer (fail closed by default), `enforce_sandbox` is the
  fail-closed gate an autonomous `Run` must pass, `NoSandboxBackend` represents
  "no real sandbox" and denies execution, and `TestSandboxBackend` provides
  scripted availability for tests. Sandbox (files/network/environment) stays
  independent from process lifecycle/containment by design (design.md Decision 7).
- `workspace_run_command` is now backed by `ProcessManager` (start + poll +
  kill), and the governed executor's `Run` fails closed with
  `sandbox_unavailable` until a real `SandboxBackend` is registered.

## Phase 6: Web, Search, and Glob Hardening

- `web_access.rs`: a pure, hermetic decision surface (no network I/O, no HTTP
  client). `WebAccessPolicy` validates scheme/credentials/host/resolved
  addresses/ports/redirects/host overrides before any connection; `PinnedConnector`
  binds a validated network target to the actual connection and verifies the
  actual peer IP (preventing DNS rebinding); `WebAccessDecision` /
  `WebAccessDenyReason` produce structured, machine-readable failures such as
  `web_access_denied`.
- `web_fetch_url` fails closed: arbitrary-URL fetching is denied before any
  connection (structured `web_access_denied`), no ambient proxy is adopted, and
  redirects are not auto-followed (design Decision 8 / tasks 6.1–6.3).
- `search.rs`: `SearchEngine` replaces the wildcard pseudo-regex and the
  unstable first-N-files traversal with real `regex`, `globset`, and `ignore`
  semantics. `workspace_search_text` and `workspace_glob_files` use it, produce
  deterministically ordered results, respect `.gitignore`, and report
  truncation honestly (`truncated=true` + reason) instead of silently dropping
  results.

## Phase 7: First New Foundation Tools

- `image_artifact.rs`: workspace-scoped `view_image`. Returns a reference-based
  `ImageArtifact` (canonical on-disk path as the controlled reference, optional
  capped payload); validates extension and MIME from magic bytes, parses header
  dimensions for common formats, enforces metadata limits, and never decodes
  pixels.
- `mcp_resources.rs`: separate list-resources, list-resource-templates, and
  read-resource read-only control entries that call a source-bound
  `McpTransport`. Registry snapshot data is discovery metadata only; transport
  responses are treated as untrusted content with bounded fields, and
  `ResourceTemplate` is an independent type from MCP prompt templates.
- `tool_search_elevation.rs`: `ToolSearchElevator` searches the registry for
  `Deferred` descriptors, returns stable candidates, and elevates a selected
  candidate's full schema into the current turn's `TurnToolView` via
  `elevate_from_registry`; elevation is validated against the current snapshot /
  source revision, expires at turn end / source replacement / reload, and is
  recorded in trace.

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

## Ports: Wired vs Still Open

The contract-only boundaries defined in `tool_runtime.rs` are now mostly
consumed by production paths:

| Type | Status |
|---|---|
| `ToolDispatcher`, `InvocationOrigin`, `PrimitiveToolHandler` | **Wired** — phase 3 (`dispatcher.rs`, governed executor) |
| `PendingControlRequest` (session/run/turn/call binding, digests, nonce, version, expiry) | **Wired** — phase 4 (Ask/approval + control plane) |
| `SandboxBackend`, `ProcessBackend` | **Wired** — phase 5 (`sandbox.rs` gate + `process.rs` `ProcessManager`); a real `SandboxBackend` implementation for autonomous Run is still open |
| `WebResolver` | **Wired** — phase 6 (`web_access.rs` policy/pinned connector) |
| `McpTransport` | **Wired** — phase 7 (`mcp_resources.rs`) |
| `FakeClock`, `FakeResolver`, `FakeMcpTransport` | test harness |

`ToolOutcome` splits `execution_status` (`Ok` / `Error` / `Cancelled`) from an
orthogonal `control_outcome`, with `from_legacy_result` / `into_legacy_result`
adapting the legacy `ToolResult` string envelope. A pending control request has
no provider-consumable result, so `into_legacy_result` fails closed with
`control_outcome_pending` rather than fabricating one.

## Two Remaining Integrator Notes

1. **Session-context threading for Ask.** `GovernedToolExecutor` is a harness
   seam: its `LegacyCompatiblePolicyEvaluator` reproduces legacy direct-execution
   behavior, and the executor does not yet thread real session/run/turn facts
   into `dispatch_governed` (phase-3 review P3-4). Before Ask/approval can run in
   a real turn, the runtime must call `dispatch_governed` with the live
   session/run/turn context so `PendingControlRequest` binding, `waiting_user`
   suspension, and exactly-one-terminal-result resume happen with real facts
   rather than a migration evaluator. The graph wait/resume and control-plane
   surface already exist; what remains is the runtime turn-loop wiring.
2. **Real `SandboxBackend` for `Run` — adjudicated 2026-08-04 (PA-076 remaining
   item 4).** This note is **resolved as: fail-closed is the design-compliant
   terminal state for this card.** With `NoSandboxBackend` + `SandboxSupportMatrix`
   registered, autonomous `Run` returns `sandbox_unavailable`, which is exactly
   what design.md Decision 7 and the process-tool-lifecycle spec prescribe:
   「无人值守 `Run` 只在真实 sandbox 可用时启用；无 sandbox backend 的平台 fail
   closed」and「runtime SHALL fail closed 并返回 `sandbox_unavailable`；SHALL NOT
   退化为普通 shell 或只使用 denylist/process group」. An unsandboxed run is only
   ever a per-invocation host approval, flagged high-risk in result and trace —
   never a silent downgrade (design.md Non-Goals also forbids using a Job Object
   or process group as a sandbox/approval substitute; they are defense-in-depth
   containment only).
   The feasibility evaluation
   (`management/task-system/02_REVIEWS/2026-08-04-pa076-sandbox-backend-evaluation.md`)
   confirms `windows-sys` 0.61.2 is already in the dependency tree (transitive)
   and exposes the complete Job Object API under the `Win32_System_JobObjects`
   feature (`CreateJobObjectW`, `SetInformationJobObject` with
   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and breakaway flags off,
   `AssignProcessToJobObject`, `TerminateJobObject`), so a real **Windows Job
   Object containment** backend is feasible and is split out as follow-up card
   **PA-077**; a **full sandbox** (workspace file/network containment + Job
   Object) is a larger, separate follow-up. Until those land, fail-closed remains
   the correct and compliant terminal state; the containment strategy decision
   (Windows non-breakaway Job Object vs best-effort process group) stays recorded
   in `sandbox.rs`.

## Code Layout

| File | Change |
|---|---|
| `crates/pony-agent-core/src/agent/tools.rs` | `ToolDescriptor`, `ToolRegistrySnapshot`, `ToolSurface`, typed `ToolPermissionDeclaration`, `ToolOutcome`, slot ordering, name pass-through; `ProcessManager`-backed `workspace_run_command`; fail-closed `web_fetch_url`; `SearchEngine`-backed search/glob |
| `crates/pony-agent-core/src/agent/tool_runtime.rs` | Dispatcher / control-request / sandbox / process / resolver / MCP ports and fake harnesses |
| `crates/pony-agent-core/src/agent/dispatcher.rs` | `GovernedDispatcher` eight-step pipeline, CAS, child runner, permission aggregation |
| `crates/pony-agent-core/src/agent/budget.rs` | Atomic `BudgetLedger` and `CancellationToken` |
| `crates/pony-agent-core/src/agent/child_dispatch.rs` | Bounded child dispatch context (lineage, depth, shared budget, cycle rejection) |
| `crates/pony-agent-core/src/agent/dispatcher_composites.rs` | Governed `workspace_batch` / `workspace_gather_context` composites + `GovernedToolExecutor` |
| `crates/pony-agent-core/src/agent/governed_executor.rs` | `build_governed_executor` — the runtime's default tool executor |
| `crates/pony-agent-core/src/agent/plan_state.rs` | `PlanStore` / `PlanControlHandler` (phase 4) |
| `crates/pony-agent-core/src/agent/ask_control.rs` | Host-facing Ask adapter over `PendingControlRequest` (phase 4) |
| `crates/pony-agent-core/src/agent/process.rs` | `ProcessManager` — session-scoped process lifecycle (phase 5) |
| `crates/pony-agent-core/src/agent/sandbox.rs` | `SandboxSupportMatrix`, `NoSandboxBackend`, `TestSandboxBackend` (phase 5) |
| `crates/pony-agent-core/src/agent/web_access.rs` | `WebAccessPolicy`, `PinnedConnector`, structured deny reasons (phase 6) |
| `crates/pony-agent-core/src/agent/search.rs` | `SearchEngine` — regex / globset / ignore semantics (phase 6) |
| `crates/pony-agent-core/src/agent/image_artifact.rs` | Workspace-scoped `view_image` (phase 7) |
| `crates/pony-agent-core/src/agent/mcp_resources.rs` | MCP resource list/templates/read via `McpTransport` (phase 7) |
| `crates/pony-agent-core/src/agent/tool_search_elevation.rs` | Deferred ToolSearch elevation (phase 7) |
| `crates/pony-agent-core/src/agent/graph.rs` | `ask_waits` binding, `bind_ask_wait` / `resume_ask_wait`, `waiting_user` suspension |
| `crates/pony-agent-core/src/agent/control_plane/ask_plan_commands.rs` | Ask/Plan/graph-Ask control-plane commands |
| `crates/pony-agent-core/src/agent/provider.rs` | Payload generation reads registry projection; tool-count inference removed |
| `crates/pony-agent-core/src/agent/capability_bridge.rs` | Builtin capability permissions read descriptor facts |
| `crates/pony-agent-core/src/agent/planner.rs` | Planner tool views read `ToolSurface` |
| `crates/pony-agent-core/src/agent/runtime/mod.rs` | `AgentRuntimeBuilder` default tool executor = `build_governed_executor` |
| `src-tauri/src/lib.rs` | `list_available_tools` reads `ToolSurface`; 13 Ask/Plan/graph-Ask Tauri commands |
| `src/stores/ask.ts`, `src/stores/plan.ts`, `src/components/AskPanel.vue`, `src/components/PlanPanel.vue`, `src/types/ask-plan.ts` | Frontend Ask/Plan surface |
| `crates/pony-agent-core/tests/dispatcher_matrix.rs` | 27-test governed dispatcher matrix |

## Building and Testing on Windows

MSVC environment variables are not loaded by default. Use the wrapper:

```bash
cmd //c "scripts\run-rust-msvc.bat cargo test -p pony-agent-core --lib --target-dir target-test-exact-a agent::tools::"
```

`scripts/invoke-rust-target.ps1` (behind `npm run cargo:test:*`) pins
`--manifest-path src-tauri/Cargo.toml`, so `--lib` there targets the Tauri
crate. Core crate tests need an explicit `-p pony-agent-core`.

Current gates (as of the phases 1–7 + switch verification): core lib 672 +
dispatcher matrix 27 + `tool_router_regression` 13 + `session_regression` 5,
plus the frontend vitest suites (328 tests). Two known environment flakes are
recorded in the PA-076 task card Blockers:
`provider_registry_regression::resolve_selection_*` (stale config.rs assertion)
and an intermittent mock-server streaming flake (passes in isolation).
