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

- [x] 3.1 Implement top-level dispatcher lifecycle for origin authorization, raw validation, mutable hooks, final validation/normalization, final-argument policy and sandbox decision, budgets, execution, control outcome, telemetry, and exactly-once lifecycle hooks.
- [x] 3.2 Bind approvals and pending control requests to descriptor snapshot, final args digest, policy digest, workspace/session/run/turn/call facts; reject replay, expiry, source revision, and CAS conflicts.
- [x] 3.3 Introduce bounded child dispatch context with parent lineage, allowed-child authority, depth, and a shared atomically reserved call/time/concurrency/output budget ledger plus cycle rejection.
- [x] 3.4 Temporarily restrict legacy `workspace_batch` to explicit read-only children until it uses child dispatch; return structured rejection for stronger scopes.
- [x] 3.5 Migrate `workspace_batch`, `workspace_gather_context`, and tool-only skills to child dispatch so every child has independent mediation, permission, hooks, and telemetry.
- [x] 3.6 Aggregate typed composite permission scopes conservatively while preserving child permission evidence.
- [x] 3.7 Add dispatcher matrix tests for builtin/MCP/skill/composite, model/child/host origin, allow/deny/approval/control suspend, hook rewrite/re-authorization, cancellation, atomic budget exhaustion, and partial results.
- [x] 3.8 Run phase-3 security and code reviews; resolve all P0/P1 findings. (Review: `management/task-system/02_REVIEWS/2026-08-02-pa076-phase3-review.md`.)

## 4. Plan and Ask Control Tools

- [x] 4.1 Replace `Plan -> workspace_batch` with a session-owned, revisioned plan-state handler supporting create/replace/merge/complete-step without arbitrary child execution.
- [x] 4.2 Move local multi-path gathering off the `Plan` identity and retain it as an internal governed composite.
- [x] 4.3 Replace `Ask -> echo_input` with `PendingControlRequest` and atomic `waiting_user` graph/session/checkpoint state; preserve the originating assistant tool-call transcript.
- [x] 4.4 Add host control-plane and Tauri/frontend adapters to display, answer, cancel, expire, reload, and resume Ask requests by stable request id and CAS version.
- [x] 4.5 Add Plan revision/transition and Ask wait/reload/answer/resume/replay/cancel/expiry/no-interactive-host tests.
- [x] 4.6 Run phase-4 architecture, UX, and code reviews; resolve all P0/P1 findings. (完成：`02_REVIEWS/2026-08-05-pa076-phase4-review.md`；P0-1 Ask 端到端 resume + 4.6-P1-1/P1-2 已在 P0 修复中处理并端到端验证；4.6-P1-3 接受为迁移窗口决策并追踪)

## 5. Process Lifecycle

- [x] 5.1 Define and test the platform sandbox support matrix, sandbox policy, minimum environment, filesystem/network limits, and explicit host-approved unsandboxed fallback; fail closed for autonomous Run when no real sandbox exists.
- [x] 5.2 Decide and document the Windows non-breakaway Job Object versus platform containment implementation using a focused spike; document Unix process-group limits rather than promising complete containment. (决策记录于 `sandbox.rs`：Windows Job Object（禁 breakaway）为意图 containment，Unix process group 为 best-effort；均未宣称完整 containment，Job Object 尚未实现为完整 enforcement。)
- [x] 5.3 Implement session-scoped process manager and platform backends for start, poll, write-stdin, kill, timeout, cancel, and session shutdown with opaque session-bound handles.
- [x] 5.4 Drain stdout/stderr concurrently into bounded buffers and return truncation/observed-byte evidence.
- [x] 5.5 Migrate `Run` to the sandboxed process lifecycle while preserving short-command compatibility fields.
- [x] 5.6 Add process tests for sandbox denial, environment isolation, cross-session handle rejection, large output, interactive stdin, non-zero exit, timeout, cancellation, containment canary, output budgets, and permissions.
- [x] 5.7 Run phase-5 security, performance, and code reviews; resolve all P0/P1 findings. (完成：`02_REVIEWS/2026-08-05-pa076-phase5-review.md`；5.7-P1-1 `isolate_environment` 标志 + 5.7-P1-2 legacy Run fail-closed 已修复)

## 6. Web, Search, and Glob Hardening

- [x] 6.1 Fail closed arbitrary URL WebFetch until a pinned connector is available; add standard URL parsing and `WebAccessPolicy` validation for scheme, credentials, host, resolved IPs, ports, redirects, and host overrides.
- [x] 6.2 Implement injected resolver/pinned connector with peer-IP verification, explicit proxy contract, redirect re-resolution, total deadline, and response streaming budgets before re-enabling arbitrary URL WebFetch.
- [x] 6.3 Stream WebFetch responses under redirect/time/body/compression/header budgets and reject unsupported content types without arbitrary text decoding.
- [x] 6.4 Replace wildcard pseudo-regex and custom traversal with standard regex, glob, ignore, deterministic ordering, and explicit truncation evidence.
- [x] 6.5 Add hermetic SSRF/rebinding/peer-IP/redirect/oversize/compression/binary/proxy tests and Search/Glob regex/ignore/order/budget tests.
- [x] 6.6 Run phase-6 security, performance, and code reviews; resolve all P0/P1 findings. (完成：`02_REVIEWS/2026-08-05-pa076-phase6-review.md`；6.6-P1-1 链级总 deadline 已修复，最坏阻塞 12min→60s，P2-3 测试缺口补齐)

## 7. First New Foundation Tools

- [x] 7.1 Implement workspace-scoped `view_image` with provider modality checks, image metadata limits, and reference-based artifacts.
- [x] 7.2 Implement separate MCP list-resources, list-resource-templates, and read-resource tools through a source-bound `McpTransport`; add independent ResourceTemplate types and capability provenance.
- [x] 7.3 Complete ToolSearch candidate selection, `TurnToolView` next-hop schema elevation, source-revision invalidation, cache mutation, and trace evidence.
- [x] 7.4 Add core, host, provider, frontend, and reload tests for image artifacts, MCP transport pagination/timeout/disconnect/malformed response/source replacement, and deferred elevation expiry.
- [x] 7.5 Run phase-7 architecture, security, and code reviews; resolve all P0/P1 findings. (完成：`02_REVIEWS/2026-08-05-pa076-phase7-review.md`；7.5-P1-1 MCP 参数回显 fail-closed + 7.5-P1-2 Plan session 注入已修复)

## 8. Migration and Closeout

- [x] 8.1 Remove superseded name-derived metadata tables, unsafe execution bypasses, and compatibility code whose migration tests have passed. (完成于 `328b1d7`：`ToolCallContractView`/`ToolResultContractView`/`builtin_tool_contract_views` 投影 + 孤儿 helper + 死 `ToolRouter` 面已删；legacy `builtin_tools()`/`ToolRouter` 保留作执行/兼容输入与 characterization 门禁)
- [x] 8.2 Update runtime/tool architecture docs, roadmap, docs index, frontend type contracts, and migration notes.
- [x] 8.3 Run formatting, lint/type checks, core targeted and full Rust tests, frontend unit/build/E2E as applicable, non-Tauri harness, and strict OpenSpec validation. (完成 2026-08-05：core lib 723 + matrix 27 + tool_router_regression 13 + session_regression 5 + 前端 vitest 332 + rustfmt 0 diff + git diff --check clean + openspec strict validate valid；唯一间歇 flake 为 session attachment 时间窗口测试，隔离恒通过，与 PA-076 无关)
- [x] 8.4 Complete independent dual code review, security review, performance review, and consultant closeout with no unresolved P0/P1. (完成 2026-08-05：4 份阶段独立审核 + `02_REVIEWS/2026-08-05-pa076-84-closeout.md` consultant 裁决 PASS，无未解决 P0/P1；4.6-P1-3 接受为迁移窗口决策并追踪)
- [x] 8.5 Sync PA-076 dashboard/board/task card, review evidence, session log, canonical specs, and archive readiness. (同步已执行；**归档本身未执行**，见任务卡 "Archive Readiness (8.5)" 一节：等待 4.6/5.7/6.6/7.5 审核与 8.1/8.3/8.4 完成后再归档)
