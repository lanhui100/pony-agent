# Tasks: Add Model Monitor and Telemetry Aggregation Surface

## 1. Scope and Contracts

- [x] 1.1 Define `PA-024` as the monitor/read-plane change for telemetry aggregation and retrieval observability
- [x] 1.2 Document dependency boundaries with `PA-025` and `PA-029`
- [x] 1.3 Document explicit non-goals so this change does not absorb runtime rewrites or analytics-platform work

## 2. Aggregation Read Models

- [x] 2.1 Define overview aggregate fields for requests, tokens, cache usage, latency, and retrieval participation
- [x] 2.2 Define dimension aggregate read models for `provider`, `model`, `tool`, and `session`
- [x] 2.3 Define session drill-down payloads that connect aggregates back to trace history

## 3. Retrieval and Trace Semantics

- [x] 3.1 Define retrieval observability as a first-class monitor dimension
- [x] 3.2 Define trace display semantic categories for retrieval, build-context, model, tool, and return steps
- [x] 3.3 Define how follow-up model calls after tools remain distinguishable without duplicating turn-level summaries

## 4. Tauri and Frontend Integration

- [x] 4.1 Define Tauri summary read surfaces that power `ModelMonitorPage`
- [x] 4.2 Define Tauri session drill-down surfaces for selected-session inspection
- [x] 4.3 Define `ModelMonitorPage` information architecture for overview, pivots, and drill-down

## 5. Validation

- [x] 5.1 Document acceptance criteria for aggregate overview rendering
- [x] 5.2 Document acceptance criteria for provider/model/tool/session pivot behavior
- [x] 5.3 Document acceptance criteria for retrieval observability and trace drill-down explanation
- [x] 5.4 Confirm the spec stays within documentation/read-plane scope only
