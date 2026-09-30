# cache-hit-optimization Specification

## Purpose

规范 Pony Agent 运行时缓存命中优化的工程契约（任务卡 `PA-029`）：provider 调用级 cache telemetry（区分 `initial_request` 与 `tool_followup`）、turn 聚合与 per-call 明细并存、前缀变更原因记录，以及把高波动注释移出最早 stable prefix 的第一版前缀稳定化，同时保留 `PA-025` 的三层 build-context 观测语义。原为 delta 格式，2026-09-30 按 canonical 格式规范化（`PA-102`），语义未变。

## Requirements

### Requirement: Runtime traces must distinguish initial requests from tool follow-up requests
The system SHALL record cache-related provider usage at a per-call level and classify at least current mainline initial requests and tool follow-up requests separately.

#### Scenario: A turn makes one initial request and multiple tool follow-up requests
- **WHEN** a turn issues provider calls during one runtime loop
- **THEN** the trace SHALL preserve call-level cache usage for each provider call
- **AND** the trace SHALL distinguish whether each call was an `initial_request` or a `tool_followup`

### Requirement: Cache telemetry must remain explainable at both call and turn levels
The system SHALL preserve aggregated turn-level token usage while making the per-call cache contribution inspectable.

#### Scenario: A user inspects a turn with low apparent cache hit rate
- **WHEN** the user or audit surface reads the turn trace
- **THEN** the system SHALL expose the turn aggregate token usage
- **AND** it SHALL expose enough per-call detail to determine whether low cache hit came from the first request or later follow-up calls

### Requirement: Request assembly must expose why cache-critical prefix content changed
The system SHALL record first-pass reasons when the request prefix shape changes due to high-volatility context fields or truncation boundary shifts.

#### Scenario: Session summary text changes between adjacent turns
- **WHEN** a request differs because session-summary-derived context changed
- **THEN** the trace SHALL record a prefix mutation reason indicating that summary-driven change

#### Scenario: Native transcript truncation shifts the retained boundary
- **WHEN** a provider-native transcript boundary moves because of truncation
- **THEN** the trace SHALL record that the boundary shifted

### Requirement: The most stable request prefix must exclude avoidable high-volatility notes
The system SHALL keep avoidable high-volatility notes out of the most cache-critical stable prefix whenever the request format supports that separation.

#### Scenario: A request includes truncation and image capability notes
- **WHEN** the request builder assembles provider messages
- **THEN** the cache-critical stable prefix SHALL remain limited to the narrow stable layer
- **AND** truncation or image capability notes SHALL NOT be treated as stable-prefix content by default

### Requirement: Existing build-context observation semantics must remain valid
The system SHALL preserve the PA-025 contract that build-context observation describes the actual request sent to the provider with stable, semi-stable, and volatile layers.

#### Scenario: Build-context observation is inspected after prefix stabilization changes
- **WHEN** the runtime emits build-context observation for a turn
- **THEN** the observation SHALL still distinguish stable prefix, semi-stable context, and volatile input
- **AND** the new cache telemetry additions SHALL not collapse those layers back into one undifferentiated request description
