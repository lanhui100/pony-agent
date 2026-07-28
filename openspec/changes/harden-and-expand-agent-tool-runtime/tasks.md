## 1. Baseline and Review Gates

- [x] 1.1 Add characterization tests for product names, non-controversial schemas/outputs, aliases, provider payloads, permission projections, and legacy composite behavior; mark Ask/Plan placeholder mappings as migration-removal tests rather than future contracts. (Rust execution attempted through the prescribed target-slot script but blocked by missing MSVC `link.exe`; strict OpenSpec and diff checks pass.)
- [x] 1.2 Complete independent architecture, security, and test-plan reviews; record each P0-P3 finding and the adoption decision in the PA-076 review artifact.
- [x] 1.3 Revise proposal, design, specs, and this task list until all review P0/P1 findings are resolved and rerun strict OpenSpec validation.

## 2. Descriptor and Registry Truth Source

- [x] 2.1 Introduce module boundaries for tool contracts, descriptors, registry, dispatcher, primitive handlers, control handlers, composite handlers, control requests, sandbox, and MCP transport without changing product-visible behavior. (Foundational ports are isolated in `tool_runtime`; full dispatcher execution remains task 3.1.)
- [x] 2.2 Define structured `ToolOutcome` with finite execution status plus orthogonal control outcome; provide a compatibility adapter for the legacy `ToolResult` string envelope.
- [x] 2.3 Define persisted `PendingControlRequest`, `TurnToolView`, `SandboxBackend`, `ProcessBackend`, `McpTransport`, fake clock/handler/resolver harnesses, and their ownership boundaries.
- [x] 2.4 Define `ToolDescriptor` identity, schemas, kind, exposure, typed permission declaration, execution policy, display/search metadata, handler provenance, and source revision with fail-closed defaults.
- [x] 2.5 Build a versioned registry snapshot that rejects duplicate identities, ambiguous aliases, reserved namespace collisions, source mismatch, and cycles; adapt existing builtin definitions into descriptors.
- [x] 2.6 Migrate provider payload generation, capability registration, planner views, frontend contract views, and list-available-tools to the registry snapshot; define `TurnToolView` as the sole provider tool surface. (Legacy `builtin_tools()` remains only as an execution/compatibility input.)
- [x] 2.7 Remove tool-count inference and add registry/provider/capability projection, source-replacement, and old-persistence compatibility tests. (Rust tests now execute; see task 2.8 for results.)
- [x] 2.8 Run phase-2 architecture/code review and required Rust checks/tests before continuing. (MSVC environment resolved via `scripts/run-rust-msvc.bat`; `agent::tools::` 65 passed, `agent::tool_runtime::` 4 passed, `tool_router_regression` 13 passed. Three real phase-2 defects found and fixed: provider tool ordering regression, external-name mislabeling, and an incorrect `workspace_path_info` product-name assertion.)

## 3. Governed Dispatcher and Permissions

- [ ] 3.1 Implement top-level dispatcher lifecycle for origin authorization, raw validation, mutable hooks, final validation/normalization, final-argument policy and sandbox decision, budgets, execution, control outcome, telemetry, and exactly-once lifecycle hooks.
- [ ] 3.2 Bind approvals and pending control requests to descriptor snapshot, final args digest, policy digest, workspace/session/run/turn/call facts; reject replay, expiry, source revision, and CAS conflicts.
- [ ] 3.3 Introduce bounded child dispatch context with parent lineage, allowed-child authority, depth, and a shared atomically reserved call/time/concurrency/output budget ledger plus cycle rejection.
- [ ] 3.4 Temporarily restrict legacy `workspace_batch` to explicit read-only children until it uses child dispatch; return structured rejection for stronger scopes.
- [ ] 3.5 Migrate `workspace_batch`, `workspace_gather_context`, and tool-only skills to child dispatch so every child has independent mediation, permission, hooks, and telemetry.
- [ ] 3.6 Aggregate typed composite permission scopes conservatively while preserving child permission evidence.
- [ ] 3.7 Add dispatcher matrix tests for builtin/MCP/skill/composite, model/child/host origin, allow/deny/approval/control suspend, hook rewrite/re-authorization, cancellation, atomic budget exhaustion, and partial results.
- [ ] 3.8 Run phase-3 security and code reviews; resolve all P0/P1 findings.

## 4. Plan and Ask Control Tools

- [ ] 4.1 Replace `Plan -> workspace_batch` with a session-owned, revisioned plan-state handler supporting create/replace/merge/complete-step without arbitrary child execution.
- [ ] 4.2 Move local multi-path gathering off the `Plan` identity and retain it as an internal governed composite.
- [ ] 4.3 Replace `Ask -> echo_input` with `PendingControlRequest` and atomic `waiting_user` graph/session/checkpoint state; preserve the originating assistant tool-call transcript.
- [ ] 4.4 Add host control-plane and Tauri/frontend adapters to display, answer, cancel, expire, reload, and resume Ask requests by stable request id and CAS version.
- [ ] 4.5 Add Plan revision/transition and Ask wait/reload/answer/resume/replay/cancel/expiry/no-interactive-host tests.
- [ ] 4.6 Run phase-4 architecture, UX, and code reviews; resolve all P0/P1 findings.

## 5. Process Lifecycle

- [ ] 5.1 Define and test the platform sandbox support matrix, sandbox policy, minimum environment, filesystem/network limits, and explicit host-approved unsandboxed fallback; fail closed for autonomous Run when no real sandbox exists.
- [ ] 5.2 Decide and document the Windows non-breakaway Job Object versus platform containment implementation using a focused spike; document Unix process-group limits rather than promising complete containment.
- [ ] 5.3 Implement session-scoped process manager and platform backends for start, poll, write-stdin, kill, timeout, cancel, and session shutdown with opaque session-bound handles.
- [ ] 5.4 Drain stdout/stderr concurrently into bounded buffers and return truncation/observed-byte evidence.
- [ ] 5.5 Migrate `Run` to the sandboxed process lifecycle while preserving short-command compatibility fields.
- [ ] 5.6 Add process tests for sandbox denial, environment isolation, cross-session handle rejection, large output, interactive stdin, non-zero exit, timeout, cancellation, containment canary, output budgets, and permissions.
- [ ] 5.7 Run phase-5 security, performance, and code reviews; resolve all P0/P1 findings.

## 6. Web, Search, and Glob Hardening

- [ ] 6.1 Fail closed arbitrary URL WebFetch until a pinned connector is available; add standard URL parsing and `WebAccessPolicy` validation for scheme, credentials, host, resolved IPs, ports, redirects, and host overrides.
- [ ] 6.2 Implement injected resolver/pinned connector with peer-IP verification, explicit proxy contract, redirect re-resolution, total deadline, and response streaming budgets before re-enabling arbitrary URL WebFetch.
- [ ] 6.3 Stream WebFetch responses under redirect/time/body/compression/header budgets and reject unsupported content types without arbitrary text decoding.
- [ ] 6.4 Replace wildcard pseudo-regex and custom traversal with standard regex, glob, ignore, deterministic ordering, and explicit truncation evidence.
- [ ] 6.5 Add hermetic SSRF/rebinding/peer-IP/redirect/oversize/compression/binary/proxy tests and Search/Glob regex/ignore/order/budget tests.
- [ ] 6.6 Run phase-6 security, performance, and code reviews; resolve all P0/P1 findings.

## 7. First New Foundation Tools

- [ ] 7.1 Implement workspace-scoped `view_image` with provider modality checks, image metadata limits, and reference-based artifacts.
- [ ] 7.2 Implement separate MCP list-resources, list-resource-templates, and read-resource tools through a source-bound `McpTransport`; add independent ResourceTemplate types and capability provenance.
- [ ] 7.3 Complete ToolSearch candidate selection, `TurnToolView` next-hop schema elevation, source-revision invalidation, cache mutation, and trace evidence.
- [ ] 7.4 Add core, host, provider, frontend, and reload tests for image artifacts, MCP transport pagination/timeout/disconnect/malformed response/source replacement, and deferred elevation expiry.
- [ ] 7.5 Run phase-7 architecture, security, and code reviews; resolve all P0/P1 findings.

## 8. Migration and Closeout

- [ ] 8.1 Remove superseded name-derived metadata tables, unsafe execution bypasses, and compatibility code whose migration tests have passed.
- [ ] 8.2 Update runtime/tool architecture docs, roadmap, docs index, frontend type contracts, and migration notes.
- [ ] 8.3 Run formatting, lint/type checks, core targeted and full Rust tests, frontend unit/build/E2E as applicable, non-Tauri harness, and strict OpenSpec validation.
- [ ] 8.4 Complete independent dual code review, security review, performance review, and consultant closeout with no unresolved P0/P1.
- [ ] 8.5 Sync PA-076 dashboard/board/task card, review evidence, session log, canonical specs, and archive readiness.
