# Design

## HTTP model

- Provider HTTP calls SHALL migrate from `reqwest::blocking` to async `reqwest`.
- Tools HTTP calls (`web_fetch_url` path) SHALL also migrate from `reqwest::blocking` to async `reqwest`.
- Streaming and non-streaming followup paths SHALL share a consistent async transport boundary.
- `pony-agent-core/Cargo.toml` SHALL remove `blocking` feature from `reqwest` dependency after migration.

## Compatibility

- Existing frontend/runtime event semantics (`turn:delta`, `turn:output_end`, terminal events) SHALL remain contract-compatible.

## Cancellation and retry

- Timeout, retry, and cancellation semantics SHALL be implemented in async-native form rather than wrapped around blocking calls.
