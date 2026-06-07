# Design: Skills Registry Bridge

## Overview

The system should treat skills as a capability-composition layer above the unified capability registry. Skills must not become a second planner, a second runtime loop, or a host-private escape hatch.

The design is intentionally split into four concerns:

1. skill manifest and registry model
2. composition and invocation boundary
3. host/planner read-write surfaces
4. observability and audit

This keeps skills aligned with the normalized capability facts introduced in `PA-020` and avoids reintroducing transport- or host-specific coupling.

## V1 Delivery Slice

The first delivery slice should stay deliberately narrow:

1. define skill registry models and source snapshots
2. adapt skill sources through the existing control-plane/capability-registry ingress family
3. expose host/control-plane read surfaces for listing and inspecting skills
4. resolve and execute only tool-composed skills in runtime `v1`
5. keep `resource` / `prompt_template` references inspectable and observable, but not yet executable

This avoids turning `PA-021` into planner redesign, workflow mode, or a second runtime scheduler.

## Core Model

### Skill Manifest

Each reusable skill should be represented by a normalized manifest record.

Suggested fields:

- `skillId`
- `sourceId`
- `label`
- `description`
- `inputSchemaSummary`
- `safetyClass`
- `visibility`
- `requiresApproval`
- `hostMediated`
- `permissionScope`
- `composedCapabilityRefs`
- `observabilityTags`

### Skill Registry Entry

Planner/runtime should consume normalized skill facts rather than raw manifest bodies or host-private script details.

Minimum requirements:

- each skill preserves provenance through `sourceId`
- each skill declares which normalized capabilities it may compose
- each skill preserves `composedCapabilityRefs` and `composedCapabilityKinds`
- each skill exposes enough metadata for planner selection without exposing implementation internals
- each skill is inspectable through the same family of read surfaces as capabilities
- each skill aggregates approval / host-mediation facts conservatively from referenced capabilities

## Composition Rules

### Capability Composition

Skills should compose already-normalized capabilities rather than bypassing the registry.

Requirements:

- a skill may reference `tool`, `resource`, and `prompt_template` capabilities
- a skill must preserve the semantic distinction between referenced capability kinds
- a skill may add orchestration metadata, but must not erase approval or permission facts inherited from referenced capabilities
- `v1` runtime execution may reject or defer non-tool compositions, but it must do so explicitly rather than flattening them into pseudo-tools

### Invocation Boundary

Skill invocation should be modeled as a normalized high-level action rather than direct host-private code execution.

Requirements:

- runtime should resolve a skill into a validated composition plan or invocation action
- planner should not require knowledge of the skill's internal execution steps
- skills must not introduce a second scheduler independent of graph/runtime execution flow
- the first implementation slice should attach at `control_plane -> snapshot ingress -> capability registry`, not by injecting skill state into planner/graph internals first

## Query and Control Surfaces

### Read APIs

The host/control-plane surfaces should support:

- `list_skills(filter)`
- `inspect_skill(skillId)`
- optionally unified capability reads that can include skills in the future without changing planner semantics

Read surfaces should stay host-agnostic and expose normalized outputs only.

### Internal Write API

The first production write surface should remain inside the Rust host/control-plane.

Recommended contract:

- apply one normalized skill source snapshot at a time
- accept source metadata plus the source's normalized skill list
- replace the previous skill set for the same `sourceId` atomically
- validate manifests before updating runtime-visible state
- synchronize control-plane read views and runtime skill resolution from the same snapshot
- keep the first ingress adapter structurally aligned with the existing capability snapshot update path

## Planner and Runtime Boundary

Planner should consume only:

- skill existence
- description and input summary
- safety and permission facts
- provenance and visibility
- high-level composed-capability facts

Planner should not consume:

- host connector specifics
- raw manifest bodies
- transport details of underlying MCP capabilities
- imperative execution scripts

Runtime may consume normalized skill invocation actions, but must do so through the same graph/runtime control flow already in place.

For `v1`, runtime execution should only require tool-composed skills. Mixed compositions that include `resource` or `prompt_template` may remain discoverable and inspectable without becoming executable in the same slice.

## Observability and Audit

The bridge should emit enough telemetry to answer:

- which skill was selected
- which source provided the skill
- which normalized capabilities the skill invoked or expanded
- which capability kinds were referenced
- whether failure occurred at skill resolution, underlying capability execution, or permission gating

Skill observability should extend the existing monitor summary/drilldown chain rather than creating a separate telemetry system.

## Dependencies and Non-Goals

This change depends on existing boundaries:

- `PA-019` planner decision boundary
- `PA-020` capability registry and bridge
- `PA-024` monitoring read-plane

This change explicitly does not:

- redesign graph progression semantics
- implement lifecycle hooks (`PA-022`)
- implement workflow mode (`PA-026`)
- introduce a marketplace or distribution platform
- allow arbitrary host-private code paths outside the normalized registry boundary
