# Tasks: Add Cache Hit Telemetry and Prefix Stabilization

## 0. Execution Freeze

- [x] 0.1 Freeze scope to current-phase cache telemetry, request kind, mutation reasons, and first-pass prefix stabilization
- [x] 0.2 Explicitly exclude auto-compaction, planner/executor dual-session refactor, and sub-agent cache reuse
- [x] 0.3 Preserve the `PA-025` three-layer build-context observation contract unchanged

## 1. Telemetry Contract

- [x] 1.1 Add shared telemetry types for `ProviderRequestKind` and `ProviderCallCacheRecord`
- [x] 1.2 Extend turn trace persistence with `provider_call_records` using backward-compatible defaults
- [x] 1.3 Record per-call cache telemetry in runtime paths while preserving turn-level aggregates
- [x] 1.4 Classify at least `initial_request` and `tool_followup` in both sync and stream flows
- [x] 1.5 Keep provider usage extraction focused on raw usage parsing instead of request classification

## 2. Prefix Mutation Observability

- [x] 2.1 Freeze first-pass `PrefixMutationReason` values:
  - `session_summary_changed`
  - `run_goal_changed`
  - `long_term_memory_changed`
  - `image_note_changed`
  - `truncation_note_changed`
  - `history_boundary_shifted`
  - `native_transcript_boundary_shifted`
- [x] 2.2 Record mutation reasons during request assembly for normalized-input and provider-native paths
- [x] 2.3 Persist mutation reasons in turn trace and session history without breaking legacy trace recovery

## 3. First-pass Prefix Stabilization

- [x] 3.1 Narrow the cache-critical stable prefix to:
  - base system prompt
  - provider capability note
  - stable tool definition export
- [x] 3.2 Keep high-volatility notes out of the earliest stable layer by default:
  - session summary
  - graph name / conversation linkage notes
  - run goal
  - long-term memory summary or status notes
  - image capability note
  - truncation note
- [x] 3.3 Correct fallback observation derivation so dynamic system/developer notes are not misclassified as stable prefix
- [x] 3.4 Preserve `PA-025` stable/semi-stable/volatile observation semantics exactly

## 4. Validation

- [x] 4.1 Add runtime tests for per-call cache telemetry with request kind classification
- [x] 4.2 Add context tests for mutation reason recording and stable-prefix exclusion of image/truncation notes
- [x] 4.3 Add session persistence tests for new per-call fields and mutation reasons
- [x] 4.4 Add regression coverage proving legacy trace/session payloads default missing new fields safely
- [x] 4.5 Add frontend store tests that preserve `requestKind` and `prefixMutationReasons` from host payloads

## 5. Task System Sync

- [x] 5.1 Link `PA-029` to this OpenSpec change
- [x] 5.2 Update dashboard/task board ordering so `PA-029` feeds `PA-024`
- [x] 5.3 Write implementation freeze and verification matrix back into the active task card and session log
