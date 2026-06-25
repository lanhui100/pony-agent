# Tasks

- [ ] Freeze provider boundary inputs/outputs
- [ ] Freeze tools HTTP boundary for `web_fetch_url` path
- [ ] Update `pony-agent-core/Cargo.toml`: remove `reqwest` `blocking` feature, add async feature
- [ ] Replace blocking HTTP client paths with async reqwest (provider.rs)
- [ ] Replace blocking HTTP client paths with async reqwest (tools.rs)
- [ ] Convert streaming followup paths to async transport
- [ ] Preserve token usage, retry, and timeout semantics
- [ ] Update blocking-dependent `#[cfg(test)]` tests to async equivalents
- [ ] Remove all `reqwest::blocking` imports and dead code
- [ ] Validate event compatibility against runtime/front-end tests
