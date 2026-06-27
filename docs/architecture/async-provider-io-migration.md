# PA-066: Async Provider IO Migration

## Motivation

Provider and tool HTTP calls used `reqwest::blocking`, tying up OS threads
for the entire duration of every network round-trip. Even with the runtime
ownership split (PA-065), blocking IO prevented genuine async concurrency:
execution threads would block on I/O instead of yielding to the tokio
runtime, capping throughput and making cancellation/timeout harder to
implement.

## Design

All `reqwest::blocking` calls in `provider.rs` and `tools.rs` were replaced
with async `reqwest` client calls. Since the call sites are still invoked
from synchronous context (the `AgentRuntime` is not fully async), a shared
`block_on` bridge was introduced.

### `runtime_helper::block_on`

```rust
pub fn block_on<F: Future>(f: F) -> F::Output {
    match Handle::try_current() {
        Ok(handle) => handle.block_on(f),
        Err(_) => {
            static RUNTIME: OnceLock<Runtime> = OnceLock::new();
            RUNTIME.get_or_init(|| Runtime::new().expect("...")).block_on(f)
        }
    }
}
```

If a tokio runtime is already active on the thread (common in Tauri context),
it reuses it; otherwise it falls back to a static lazily-created runtime.
This avoids spawning a new OS thread per provider call.

### Data Flow

```
Synchronous caller (AgentRuntime method)
       │
       ▼
  runtime_helper::block_on()
       │
       ▼
  async reqwest::Client
       │
       ▼
  Provider / Tool HTTP call
       │
       ▼
  Future resolves → back to sync caller
```

## Key Decisions

- **`block_on` bridge over full async migration**: Migrating the entire
  `AgentRuntime` to async in one step would be too invasive. The bridge
  allows incremental per-path asyncification while keeping the public API
  synchronous.
- **Single shared `runtime_helper` module**: Both `provider.rs` and
  `tools.rs` had duplicated `block_on` logic; extracting it into
  `runtime_helper.rs` eliminated the duplication and gave PA-067/PA-068 a
  single point of evolution.
- **`reqwest` feature cleanup**: Removed `blocking` feature, kept `json`,
  `native-tls`, `charset`. Added explicit `tokio` dependency to the
  manifest.

## Code Layout

| File | Change |
|---|---|
| `crates/pony-agent-core/src/agent/provider.rs` | Migrated `reqwest::blocking` → async `reqwest` via `block_on` |
| `crates/pony-agent-core/src/agent/tools.rs` | Migrated `web_fetch_url` and other HTTP calls to async |
| `crates/pony-agent-core/src/agent/runtime_helper.rs` | New file: `block_on()` and `TestRuntimeGuard` |
| `crates/pony-agent-core/Cargo.toml` | `reqwest` removed `blocking`, added `tokio`; provider/tool test deps updated |
