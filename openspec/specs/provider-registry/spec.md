# provider-registry Specification

## Purpose

锁定 `ProviderRegistryStore::resolve_selection` 在请求的 provider/model id 不存在时的可观测行为，使回归测试反映当前（`a2636da` 之后）的 dedupe + fallback 语义，而不是过期的基线。`a2636da` 引入模型去重后，同一 provider 内共享 `model_uniqueness_key` 的模型只保留首个，被移除的重复模型若为选中模型，`selected_model_id` 会重定向到保留模型，因此 `resolve_selection` 解析到保留模型的参数（如 `max_output_tokens=4096`），旧的 64000 基线不可达。

## Requirements

### Requirement: Fallback to persisted selection

When `resolve_selection(provider_id, model_id)` is called with ids that do not match any provider/model, the resolution SHALL fall back to the persisted `selected_provider_id` and the selected provider's `selected_model_id`, in that order, before falling back to the first provider / first model.

#### Scenario: Unknown provider and model ids

- GIVEN a provider registry with a persisted selected provider and selected model
- WHEN `resolve_selection` is called with ids that match no provider/model
- THEN the resolution SHALL fall back to the persisted `selected_provider_id`
- AND the resolution SHALL then fall back to that provider's `selected_model_id`
- AND only when neither exists SHALL it fall back to the first provider / first model

### Requirement: Dedupe affects the resolved model

Within a provider, models sharing the same model value (`model_uniqueness_key`) SHALL be deduplicated to the first occurrence; if the removed duplicate was the selected model, `selected_model_id` SHALL be redirected to the kept model. `resolve_selection` therefore resolves to the kept model's parameters.

#### Scenario: Duplicate model removed

- GIVEN a provider whose two models share the model value `claude-3-7-sonnet-latest`
- WHEN the registry is normalized with dedupe
- THEN only the first occurrence SHALL be kept
- AND if the removed duplicate was selected, `selected_model_id` SHALL point to the kept model
- AND `resolve_selection` SHALL resolve to the kept model's parameters

### Requirement: Test baseline reflects post-dedupe state

The regression test `resolve_selection_falls_back_to_selected_provider_and_model` SHALL assert `max_output_tokens == 4096` for a provider whose two models share the model value `claude-3-7-sonnet-latest` (kept model `beta-chat`, 4096), documenting that the 64000 (`DEFAULT_MODERN_MAX_OUTPUT_TOKENS`) baseline is unreachable after `a2636da` dedupe.

#### Scenario: Regression test asserts post-dedupe baseline

- GIVEN the regression test `resolve_selection_falls_back_to_selected_provider_and_model`
- WHEN the test runs against the post-`a2636da` registry
- THEN the assertion SHALL expect `max_output_tokens == 4096`
- AND the test SHALL pass with `npm run cargo:test:exact -- --workspace --test provider_registry_regression`

## Acceptance

- `npm run cargo:test:exact -- --workspace --test provider_registry_regression` passes.
- `npm run cargo:check:shared` passes.
- No production code in `crates/pony-agent-core` is modified by this change.