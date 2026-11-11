# 2026-06-01 Session 56 - PA-029 实现冻结与验证矩阵

## 本次目标

将 `Mendel / Ohm / Mencius` 三路 brief 固化为主线程可执行的实现冻结版，避免 `PA-029` 在实现阶段继续漂移。

## 本次动作

1. 回写 `PA-029` 当前卡，补全：
   - 第一版实现冻结边界
   - 子智能体冻结结论
   - 最小实现顺序
   - 最小验证矩阵
2. 回写 OpenSpec `tasks.md`，把高层任务细化为有依赖顺序的执行项
3. 回写 OpenSpec `design.md`，新增：
   - `Execution Freeze`
   - `Implementation Order`
   - `Verification Strategy`

## 冻结后的核心结论

- 当前阶段只做四件事：
  - call-level cache telemetry contract
  - `initial_request` / `tool_followup`
  - `PrefixMutationReason`
  - 第一版 stable prefix 收窄
- 明确不做：
  - auto-compaction
  - planner / executor 双会话
  - 子代理缓存复用体系
- `PA-025` 的三层 build-context observation 契约保持不变：
  - `stablePrefixText`
  - `semiStableContextText`
  - `volatileInputText`
- 第一版 stable prefix 仅保留：
  - base system prompt
  - provider capability note
  - stable tool definition export

## 冻结后的实现顺序

1. `telemetry.rs`：共享类型
2. `session.rs`：trace 持久化字段
3. `runtime.rs`：per-call telemetry 与 request kind
4. `context.rs`：mutation reason 与 prefix placement
5. `provider.rs`：fallback observation 修正
6. 定向测试与 persistence 回归

## 冻结后的最小验证矩阵

- Rust / runtime：
  - `run_turn_records_call_level_cache_telemetry_with_request_kinds`
  - `start_turn_stream_records_call_level_cache_telemetry_with_request_kinds`
- Rust / context：
  - `build_request_records_prefix_mutation_reasons_for_session_summary_and_run_goal_changes`
  - `build_request_records_boundary_shift_reasons_for_history_and_native_transcript_truncation`
  - `build_request_keeps_image_and_truncation_notes_out_of_stable_prefix`
- Rust / session：
  - `file_backend_roundtrip_restores_call_level_cache_trace_fields`
- Rust / regression：
  - `legacy_turn_trace_history_defaults_missing_call_level_fields`
- Frontend / store：
  - `stores call-level model hops with request kinds from host payload`
  - `preserves prefix mutation reasons on completed trace history`

## 当前结果

- `PA-029` 已从“有方向”推进到“实现边界已冻结”
- OpenSpec 已具备可直接指导代码实现的顺序和验证口径
- 下一步可以直接进入代码实现，不需要再重复讨论本阶段范围

## 断点续跑提示

恢复时先看：

- `management/task-system/03_TASKS/PA-029-establish-cache-hit-telemetry-and-first-pass-prefix-stabilization.md`
- `openspec/changes/archive/2026-06-01-add-cache-hit-telemetry-and-prefix-stabilization/tasks.md`
- `openspec/changes/archive/2026-06-01-add-cache-hit-telemetry-and-prefix-stabilization/design.md`
