# Provider Registry — Fallback Resolution Test Baseline

## Purpose

Lock the observable behavior of `ProviderRegistryStore::resolve_selection` when the
requested provider/model ids do not exist, so that the regression test reflects the
current (post-`a2636da`) dedupe + fallback semantics instead of a stale baseline.

## Requirements

### REQ-1: Fallback to persisted selection

When `resolve_selection(provider_id, model_id)` is called with ids that do not match
any provider/model, the resolution SHALL fall back to the persisted
`selected_provider_id` and the selected provider's `selected_model_id`, in that order,
before falling back to the first provider / first model.

### REQ-2: Dedupe affects the resolved model

Within a provider, models sharing the same model value
(`model_uniqueness_key`) are deduplicated to the first occurrence; if the removed
duplicate was the selected model, `selected_model_id` is redirected to the kept
model. `resolve_selection` therefore resolves to the kept model's parameters.

### REQ-3: Test baseline reflects post-dedupe state

The regression test `resolve_selection_falls_back_to_selected_provider_and_model`
SHALL assert `max_output_tokens == 4096` for a provider whose two models share the
model value `claude-3-7-sonnet-latest` (kept model `beta-chat`, 4096), documenting
that the 64000 (`DEFAULT_MODERN_MAX_OUTPUT_TOKENS`) baseline is unreachable after
`a2636da` dedupe.

## Acceptance

- `npm run cargo:test:exact -- --workspace --test provider_registry_regression` passes.
- `npm run cargo:check:shared` passes.
- No production code in `crates/pony-agent-core` is modified by this change.