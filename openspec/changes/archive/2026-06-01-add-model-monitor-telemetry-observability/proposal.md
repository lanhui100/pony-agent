# Proposal: Add Model Monitor and Telemetry Aggregation Surface

## Why

Pony Agent has already established the core context-state split:

- `PA-018` completed retrieval-first runtime consumption
- `PA-025` owns `RetrievedContextState -> prompt/request` mapping and build-context explanation
- `PA-029` establishes cache-hit telemetry and first-pass prefix stabilization

What is still missing is the read surface that turns those runtime signals into an operator-facing monitoring product.

Today:

- retrieval state is inspectable in local trace details but not aggregated into a monitoring read model
- trace panels expose step-level data, but the semantic contract for what a model/tool/retrieval step means is still implicit
- cache and token data are mostly consumed turn-by-turn rather than as session/provider/model/tool aggregates
- `ModelMonitorPage` exists as a frontend entry point, but `PA-024` has not yet defined what telemetry contract it should consume from Tauri

This leaves the system with raw observability ingredients but without a coherent monitoring plane.

## What Changes

This change defines `PA-024` as the model monitoring and telemetry aggregation surface for the current runtime:

- define retrieval observability as a first-class monitoring dimension
- define explicit trace display semantics for retrieval, model, tool, and return phases
- define provider/model/tool/session-level telemetry aggregation read models
- define Tauri commands/events/read surfaces that feed `ModelMonitorPage`
- define frontend information architecture for `ModelMonitorPage`, including aggregate views and drill-down into session trace history

## Scope

In scope:

- telemetry read-model contracts for session, provider, model, and tool aggregation
- retrieval observability read surfaces built on top of existing retrieval/build-context data
- trace display semantics for how runtime steps are labeled, grouped, and explained in monitor UI
- Tauri-side integration contract that exposes `ModelMonitorPage` data without redefining runtime execution
- acceptance criteria for monitor landing states, aggregate cards/tables, and trace drill-down behavior

Out of scope:

- changing provider runtime behavior or request assembly logic
- replacing `PA-025` build-context semantics
- redefining `PA-029` cache telemetry fields
- adding alerting, anomaly detection, or background analytics pipelines
- introducing remote telemetry backends, multi-user dashboards, or cross-device sync
- redesigning the general app shell outside the `ModelMonitorPage` monitor flow

## Acceptance Criteria

`PA-024` is complete when the spec and design define a monitor surface where:

- a user can read session-level aggregates for token usage, cache usage, latency, and request counts
- a user can pivot those aggregates by provider, model, and tool
- a user can inspect retrieval observability alongside trace history rather than as a disconnected debug panel
- trace semantics clearly distinguish retrieval preparation, model calls, tool calls, follow-up model calls, and terminal return
- `ModelMonitorPage` has a Tauri-backed contract for summary read models and session drill-down payloads
- non-goals are explicit so the work does not absorb runtime rewrites or analytics platform work

## Impact

- affects `telemetry`, `runtime`, `session`, `trace`, and Tauri/frontend read boundaries
- consumes `PA-029` cache telemetry as an input rather than redefining it
- consumes `PA-025` build-context and retrieval explanation surfaces as an input rather than replacing them
- establishes the product-facing read plane for future diagnostics and monitoring work
