# Tasks: Add Skills Registry Bridge

## 1. Change Definition

- [ ] 1.1 Define `PA-021` as the capability-composition change above the unified capability registry
- [ ] 1.2 Document dependency boundaries with `PA-019`, `PA-020`, `PA-024`, and downstream `PA-022` / `PA-026`
- [ ] 1.3 Document explicit non-goals so this change does not absorb hooks, workflow mode, marketplace, or planner redesign

## 2. Skill Model

- [ ] 2.1 Define normalized skill manifest and registry entry models
- [ ] 2.2 Define how skill provenance, visibility, safety, and permission facts are represented
- [ ] 2.3 Define how skills reference and compose normalized capability kinds without flattening their semantics

## 3. Mapping and Invocation Boundary

- [ ] 3.1 Define skill composition rules across `tool`, `resource`, and `prompt_template`
- [ ] 3.2 Define normalized skill invocation / expansion result and failure contracts
- [ ] 3.3 Define runtime resolution rules that keep skills inside the existing graph/runtime execution flow

## 4. Host and Planner Surface

- [ ] 4.1 Define host control-plane read contracts for listing and inspecting skills
- [ ] 4.2 Define internal write/snapshot contracts for registering skill sources atomically
- [ ] 4.3 Define planner consumption rules that use only normalized skill facts

## 5. Permission and Observability

- [ ] 5.1 Define how skills inherit or aggregate approval and permission requirements from referenced capabilities
- [ ] 5.2 Define observability requirements for skill selection, composed capability usage, and failure layering
- [ ] 5.3 Define audit/readout requirements that reuse the existing monitor summary/drilldown chain

## 6. Validation

- [ ] 6.1 Write acceptance criteria proving skills remain a composition layer rather than a second scheduler
- [ ] 6.2 Write acceptance criteria proving skills reuse the unified capability registry boundary
- [ ] 6.3 Write acceptance criteria for permission/observability behavior across composed capabilities
