# Tasks: Add Skills Registry Bridge

## 1. Spec and Boundary Sync

- [x] 1.1 Define `PA-021` as the capability-composition change above the unified capability registry
- [x] 1.2 Document dependency boundaries with `PA-019`, `PA-020`, `PA-024`, and downstream `PA-022` / `PA-026`
- [x] 1.3 Document explicit non-goals so this change does not absorb hooks, workflow mode, marketplace, or planner redesign
- [x] 1.4 Complete an independent spec review and adopt the `v1` scope tightening

## 2. Registry Model and Ingress Adapter

- [x] 2.1 Add normalized Rust skill source snapshot / skill descriptor models
- [x] 2.2 Add a control-plane ingress adapter that applies one skill source snapshot atomically and keeps runtime/control-plane registries in sync
- [x] 2.3 Define and test how skill provenance, visibility, approval, host-mediation, and permission summaries are aggregated from referenced capabilities

## 3. Host Read Surface

- [x] 3.1 Add host/control-plane read contracts for `list_skills` and `inspect_skill`
- [x] 3.2 Keep skill read surfaces host-agnostic and aligned with the existing capability inspection family
- [x] 3.3 Add read-plane tests proving refreshed source snapshots replace stale skills for the same `sourceId`

## 4. Runtime Resolution and V1 Execution

- [x] 4.1 Add normalized skill resolution / invocation result contracts and failure layering
- [x] 4.2 Implement `v1` runtime resolution for tool-composed skills without introducing a second scheduler
- [x] 4.3 Keep `resource` / `prompt_template` references inspectable but non-executable in `v1`, with explicit failure/unsupported behavior

## 5. Planner and Observability

- [x] 5.1 Define planner consumption rules that use only normalized skill facts
- [x] 5.2 Add observability fields for `skillId / sourceId / composedCapabilityRefs / composedCapabilityKinds / failureLayer`
- [x] 5.3 Reuse the existing monitor summary/drilldown chain for skill usage and failure readout

## 6. Validation and Closeout

- [x] 6.1 Add tests proving skills remain a composition layer rather than a second scheduler
- [x] 6.2 Add tests proving the first bridge slice reuses the unified capability registry boundary
- [x] 6.3 Add tests covering permission aggregation and failure layering across composed capabilities
- [x] 6.4 回写任务卡、review 文档、日志与验收证据
