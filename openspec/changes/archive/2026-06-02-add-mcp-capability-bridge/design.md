# Design: MCP Capability Bridge

## Overview

The system should treat MCP as a capability-ingress layer that normalizes external capabilities into Pony Agent's internal capability facts. MCP must not become a new execution loop, planner policy engine, or host-specific side channel.

The design is intentionally split into four concerns:

1. MCP source discovery
2. capability normalization
3. capability consumption
4. permission and observability boundaries

This keeps MCP protocol details below planner/runtime decision layers and avoids repeating the same integration logic across hosts.

## Core Model

### MCP Capability Source

Each MCP-backed source should be represented as an explicit source record.

Suggested fields:

- `sourceId`
- `displayName`
- `transportKind`
- `serverIdentity`
- `availability`
- `declaredCapabilities`
- `permissionProfile`
- `updatedAtMs`

`availability` should at minimum distinguish:

- `available`
- `degraded`
- `unreachable`
- `disabled`

### Normalized Capability Record

Planner/runtime should consume normalized capability facts rather than raw MCP payloads.

Suggested fields:

- `capabilityId`
- `sourceId`
- `kind`
- `label`
- `description`
- `invocationMode`
- `inputSchemaSummary`
- `safetyClass`
- `visibility`
- `observabilityTags`

`kind` should at minimum distinguish:

- `tool`
- `resource`
- `prompt_template`

`invocationMode` should at minimum distinguish:

- `direct_tool_call`
- `read_only_fetch`
- `prompt_expansion`

### Capability Registry Contract

The registry should expose builtin and MCP-backed entries through one read model.

Minimum requirements:

- builtin capabilities and MCP-backed capabilities share a common normalized shape
- each record preserves provenance through `sourceId` and `kind`
- planner reads only normalized facts
- runtime resolves invocation through a bridge layer, not by embedding MCP protocol logic in planner

## Mapping Rules

### MCP Tools

MCP tools should map to normalized `tool` capabilities.

Requirements:

- preserve callable schema summary
- preserve provenance and permission profile
- expose enough descriptive fields for planner selection
- do not leak wire transport details into planner-facing fields

### MCP Resources

MCP resources should map to normalized `resource` capabilities.

Requirements:

- distinguish read-only retrieval semantics from executable tool semantics
- expose resource shape and access mode
- make it explicit whether runtime may fetch directly or only through host-controlled inspection

### MCP Prompt-Like Capabilities

If MCP exposes prompt templates or similar reusable prompt artifacts, they should map to normalized `prompt_template` capabilities rather than pretending to be tools.

Requirements:

- planner can see that such entries shape prompts rather than execute side effects
- runtime can request expansion through the bridge without treating them as ordinary tool calls

## Query and Control Surfaces

### Read APIs

The host/control-plane surfaces should support:

- `list_capability_sources()`
- `list_capabilities(filter)`
- `inspect_capability(capabilityId)`
- `inspect_capability_source(sourceId)`

These are read-plane contracts. They should not require hosts to understand MCP internals beyond the normalized bridge outputs.

### Internal Write API

The first production write surface should remain inside the Rust host/control-plane rather than being exposed as a frontend registration channel.

Recommended contract:

- apply one normalized source snapshot at a time
- accept `source` plus the source's normalized capability list
- replace the previous capability set for the same `sourceId` atomically
- keep an unavailable source visible even when the latest capability list is empty
- synchronize the read-plane registry and runtime execution registry from the same normalized snapshot

This keeps MCP discovery and transport logic inside the host while preserving a host-agnostic normalized bridge contract.

### Invocation APIs

The runtime-facing bridge should support:

- resolve a normalized capability into an executable/readable bridge action
- execute a `tool` capability
- fetch a `resource` capability
- expand a `prompt_template` capability

Runtime should never speak raw MCP protocol types directly in planner-visible logic.

## Planner and Runtime Boundary

Planner should consume only:

- capability existence
- capability kind
- safety/permission facts
- schema/description summaries

Planner should not consume:

- transport/session protocol details
- host connector specifics
- raw MCP payload envelopes

Runtime may consume bridge execution results, but through normalized success/error contracts.

For tool execution, runtime should resolve:

- builtin aliases first, to preserve current builtin compatibility
- then normalized MCP-backed tool capabilities by their exposed tool label

This keeps planner/runtime on capability facts while allowing host-registered MCP tools to enter the existing tool execution path.

## Permission and Observability

### Permission Boundary

The bridge must make permission state explicit before invocation.

Minimum fields:

- `requiresApproval`
- `permissionScope`
- `hostMediated`

### Failure Boundary

MCP failures should be normalized so higher layers can distinguish:

- source unavailable
- permission denied
- invocation failed
- malformed response

### Observability

The bridge should emit enough telemetry to answer:

- which capability source was used
- which normalized capability was selected
- whether the operation was tool execution, resource fetch, or prompt expansion
- whether failure was caused by transport, permission, or capability logic

## Dependencies and Non-Goals

This change depends on existing boundaries:

- `PA-015` host control plane
- `PA-018` retrieval/context read boundaries
- `PA-019` planner decision boundary

This change explicitly does not:

- redesign graph progression semantics
- implement skills orchestration
- introduce hooks as part of first-pass MCP bridge work
