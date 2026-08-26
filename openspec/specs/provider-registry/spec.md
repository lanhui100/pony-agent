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

### Requirement: Provider protocols SHALL use three canonical wire names with legacy read aliases
The canonical protocol values SHALL be `openai-completions`, `openai-responses`, and `anthropic-messages` across the persisted registry, the Tauri view boundary, and the frontend store. Deserialization SHALL additionally accept legacy values `openai` and `anthropic` (mapped to `openai-completions` and `anthropic-messages` respectively); serialization SHALL only emit canonical names. Reading a new-format file with an application build that predates this change falls back to the default registry silently — this downgrade hazard is documented as accepted in ADR 0014.

#### Scenario: Legacy registry file loads and upgrades on save
- **GIVEN** a providers.json whose provider, supportedProtocols, endpoint, or model protocol fields contain `openai` or `anthropic`
- **WHEN** the registry is loaded through either backend deserialization or frontend normalization
- **THEN** every protocol field resolves to its canonical name
- **AND** a subsequent save persists only canonical names

#### Scenario: Responses protocol routes to the responses API
- **WHEN** a resolved selection carries `openai-responses`
- **THEN** decision and tool-follow-up requests (sync and streaming) POST to `{base_url}/responses`
- **AND** at most one tool call is consumed per decision (first-only), with any additional function calls logged as dropped

### Requirement: Model-level overrides SHALL take precedence when resolving a selection
When the selected model declares a non-null protocol or non-empty base URL override, resolution SHALL prefer them over the provider-level values; otherwise the enabled provider endpoint matching the effective protocol provides the base URL before falling back to the provider base URL or protocol default. The thinking-parameter pattern and capability catalog SHALL resolve against the effective protocol family (openai family covers completions and responses).

#### Scenario: Model pinned to another protocol with its own base URL
- **GIVEN** a model with protocol `anthropic-messages` and a non-empty base-url override on an openai-completions provider
- **WHEN** the selection is resolved
- **THEN** requests target the model's base URL using anthropic-message auth derivation
- **AND** the reasoning/thinking parameter pattern follows the anthropic family for that request

### Requirement: Model catalog fetching SHALL be a guarded, authenticated GET of {base_url}/models
A dedicated command SHALL fetch model IDs given protocol, base URL, and an optional explicit API key that falls back to the stored provider key; it SHALL send Bearer auth for the openai family and x-api-key plus anthropic-version headers for anthropic, apply error-for-status, reject non-JSON bodies as errors instead of empty lists, parse `data[].id`, root arrays, and `models[].id` tolerantly, dedupe-sort with a 1000-entry cap, and never log the key. Key resolution SHALL match the requested provider id exactly — an unmatched id SHALL NOT fall back to another provider's stored key.

#### Scenario: Provider returns a JSON body without data array
- **WHEN** the endpoint responds 200 with a JSON object lacking `data`/`models` id entries or with an HTML/plain-text body
- **THEN** the command returns an explicit error rather than an empty list

## Acceptance

- `npm run cargo:test:exact -- --workspace --test provider_registry_regression` passes.
- `npm run cargo:check:shared` passes.
- No production code in `crates/pony-agent-core` is modified by this change.
