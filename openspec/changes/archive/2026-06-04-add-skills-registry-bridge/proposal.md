# Proposal: Add Skills Registry Bridge

## Why

Pony Agent now has a stable capability-ingress layer for builtin and MCP-backed capabilities, but there is still no first-class model for higher-level reusable skills. The project currently has the concept of "skills" in surrounding tooling and prompts, yet Pony Agent runtime, planner, and host surfaces do not have a formal skills registry or invocation boundary.

This gap blocks the next layer of capability work:

- exposing reusable composed capabilities to planner through normalized skill facts
- composing tools, MCP-backed capabilities, and retrieval guidance without leaking implementation details upward
- auditing skill-level execution through the same control-plane and observability surfaces used for lower-level capabilities

Without a formal bridge design, "skills" would collapse into ad hoc prompt wrappers, host-private scripts, or planner-side heuristics, which would drift across surfaces and bypass the capability registry boundary established in `PA-020`.

## What Changes

This change introduces a first-class skills registry bridge for Pony Agent:

- define skills as a capability-composition layer above the unified capability registry
- define normalized skill manifest, registry, and inspection contracts
- define skill invocation boundaries that compose existing capabilities without replacing planner or runtime
- define planner/runtime consumption rules that depend only on normalized skill facts
- define host control-plane and observability requirements for skill registration, execution, and audit

## Scope

In scope:

- skill manifest and normalized registry model
- skill-to-capability composition rules
- host control-plane read/write surfaces for skills
- skill invocation boundary and minimal lifecycle contract
- skill-level observability and audit fields

Out of scope:

- lifecycle hooks pipeline
- workflow mode / user-authored workflow engine
- full marketplace, distribution, or sandbox platform
- planner redesign or a second scheduler
- arbitrary plugin runtime or host-private script execution

## Impact

- depends on `PA-020` unified capability registry and bridge contracts
- extends planner-facing capability facts with reusable high-level skill units
- creates the dependency base for later hooks, workflow, and richer composition work without absorbing those scopes now
