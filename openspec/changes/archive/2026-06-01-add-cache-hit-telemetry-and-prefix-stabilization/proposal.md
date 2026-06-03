# Proposal: Add Cache Hit Telemetry and Prefix Stabilization

## Why

Pony Agent has already split build-context observation into stable prefix, semi-stable context, and volatile input, but the runtime still lacks an engineering-grade cache optimization surface.

Today:

- top-level token usage is mostly aggregated at the whole-turn level
- initial provider requests and tool follow-up requests are mixed together
- prompt cache behavior is hard to explain from traces
- high-volatility context notes still appear too early in the request assembly path

This makes cache hit rate look lower than expected, prevents precise diagnosis, and blocks cost-oriented runtime decisions.

Since cache hit is now treated as a first-class product metric, the next practical step is not full compaction. The next step is to make cache behavior observable and reduce avoidable prefix churn in the current request path.

## What Changes

This change introduces a first-pass cache optimization layer for the current runtime:

- add call-level cache telemetry for provider requests
- distinguish `initial_request` and `tool_followup` request classes in traces
- expose prefix mutation reasons so trace data can explain why cache reuse changed
- stabilize the earliest request prefix by keeping high-volatility notes out of the most stable layer when possible
- preserve current answer quality and existing PA-025 build-context semantics

## Scope

In scope:

- provider/runtime/trace telemetry contract for cache hit and cache miss
- request classification for current runtime flows
- first-pass prefix stabilization in request assembly
- session and trace persistence for the new telemetry fields
- Rust and frontend-facing regression coverage for the new observability semantics

Out of scope:

- full auto-compaction
- planner/executor dual-session refactor
- subagent/fork cache reuse optimization
- workflow-mode cache design
- provider-specific pricing or budget policy changes

## Impact

- affects `context`, `provider`, `runtime`, `turn_flow`, and `telemetry`
- becomes a prerequisite-quality input for `PA-024` model monitor and telemetry surfaces
- establishes the engineering baseline for later compaction work without forcing that work early
