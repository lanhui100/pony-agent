# provider-registry Delta

## ADDED Requirements

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
A dedicated command SHALL fetch model IDs given protocol, base URL, and an optional explicit API key that falls back to the stored provider key; it SHALL send Bearer auth for the openai family and x-api-key plus anthropic-version headers for anthropic, apply error-for-status, reject non-JSON bodies as errors instead of empty lists, parse `data[].id`, root arrays, and `models[].id` tolerantly, dedupe-sort with a 1000-entry cap, and never log the key.

#### Scenario: Provider returns a JSON body without data array
- **WHEN** the endpoint responds 200 with a JSON object lacking `data`/`models` id entries or with an HTML/plain-text body
- **THEN** the command returns an explicit error rather than an empty list
