# Proposal: Add MCP Capability Bridge

## Why

Pony Agent now has stable boundaries for host control plane, graph planning, runtime execution, retrieval, and monitoring, but MCP is still only a future idea in task cards and architecture notes. There is no formal contract for how MCP resources, tools, and prompt-like capabilities should enter the system.

This gap blocks the next layer of capability work:

- exposing MCP-provided tools through the same capability facts consumed by planner and runtime
- keeping MCP protocol details out of graph, planner, host, and frontend layers
- establishing permission, error, and observability boundaries before skills build on top of MCP

Without a first-class bridge design, MCP integration would likely leak protocol-specific assumptions upward into runtime, graph, or host code.

## What Changes

This change introduces a first-class MCP capability bridge for Pony Agent:

- define MCP as a capability-ingress layer rather than a scheduler/runtime layer
- define bridge contracts for MCP tools, resources, and prompt-like capabilities
- define a unified capability registry view that can expose both builtin and MCP-backed capabilities
- define planner/runtime consumption rules that depend only on capability facts, not MCP wire details
- define host control-plane and observability requirements for MCP discovery, execution, permissions, and failures

## Scope

In scope:

- MCP bridge data model and capability registry contract
- MCP tool/resource/prompt-like mapping rules
- permission and failure surface requirements
- host-agnostic control-plane read/write surfaces for MCP capability access
- observability requirements for MCP-backed capability discovery and execution

Out of scope:

- full marketplace/distribution workflow
- skill registry implementation itself
- lifecycle hooks implementation
- redesigning planner or runtime execution semantics
- implementing every possible MCP server feature in the first pass

## Impact

- affects `tools`, future `capability registry`, `planner`, `host control plane`, and observability contracts
- creates the dependency base for `PA-021` skills bridge and later `PA-022` hooks consumption
- should remain host-agnostic so Tauri, CLI, and future surfaces can share the same bridge semantics
