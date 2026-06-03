# Tasks: Add MCP Capability Bridge

## 1. Change Definition

- [x] 1.1 Define `PA-020` as the capability-ingress change for MCP-backed tools, resources, and prompt-like capabilities
- [x] 1.2 Document dependency boundaries with `PA-015`, `PA-018`, `PA-019`, and downstream `PA-021` / `PA-022`
- [x] 1.3 Document explicit non-goals so this change does not absorb skills, hooks, or planner redesign

## 2. Capability Model

- [x] 2.1 Define normalized MCP capability source and capability record models
- [x] 2.2 Define how builtin and MCP-backed capabilities coexist in one capability registry contract
- [x] 2.3 Define capability provenance, visibility, and safety/permission fields

## 3. Mapping and Execution Boundary

- [x] 3.1 Define mapping rules for MCP `tool` capabilities
- [x] 3.2 Define mapping rules for MCP `resource` capabilities
- [x] 3.3 Define mapping rules for MCP prompt-like/template capabilities
- [x] 3.4 Define normalized execution/fetch/expansion result and failure contracts

## 4. Host and Planner Surface

- [x] 4.1 Define host control-plane read contracts for listing and inspecting capability sources/capabilities
- [x] 4.2 Define runtime bridge invocation contracts that avoid leaking MCP wire details upward
- [x] 4.3 Define planner consumption rules that use capability facts only

## 5. Permission and Observability

- [x] 5.1 Define permission and approval state requirements for MCP-backed capability usage
- [x] 5.2 Define observability requirements for source usage, invocation type, and failure class
- [x] 5.3 Define degraded/unavailable source behavior without breaking host-agnostic semantics

## 6. Validation

- [x] 6.1 Write acceptance criteria proving MCP remains a capability-ingress layer rather than a scheduler layer
- [x] 6.2 Write acceptance criteria for registry unification across builtin and MCP-backed capabilities
- [x] 6.3 Write acceptance criteria for permission/failure normalization and observability
