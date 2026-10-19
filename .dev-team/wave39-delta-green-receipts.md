# wave-39-delta 绿相收据（L2-E · Executor · R8' usage 单源）

- 角色：Executor（实现者；A11 冻结测试只读，未改动任何测试断言）
- 任务板：`task-7`（W39-D3 实施 R8'）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §5（R8'）；空桩基线 `f9f400b`，红相冻结 `cf4129e`
- 环境：rustc/cargo 1.91.1，Linux
- 结论：**A11 4 passed / Exit 0；A1–A10 10 passed / Exit 0；`-D warnings` check Exit 0；三项回归逐项与基线一致**

## 1. 改动清单（唯一文件：`crates/pony-agent-core/src/bin/tui.rs`）

| 条款 | 改动 | 最终行号 |
|---|---|---|
| R8'-1 | 实现 `stream_usage_event(event_name, payload) -> Option<UiEvent>`：`event_name != "turn:output_end"` → `None`；四 token 字段全 `None` → `None`；否则 `Some(StreamUsage{..})`，缺失字段 `unwrap_or(0)`。移除 Protocol 的 `#[allow(dead_code, unused_variables)]` | 1282–1301 |
| R8'-2 | `TuiTurnSink::emit` 的 `"turn:completed" \| "turn:output_end"` 分支：原有「任一 Some 即 send StreamUsage」整段替换为 `if let Some(event) = stream_usage_event(name, &payload) { send(event) }`，随后照旧 `send(StreamCompleted)` —— `turn:completed` 不再发 usage（消除同一 persisted token 双计） | 1265–1272 |

写域审计：本 delta 仅改动上述两处（sink 分支整段替换为纯函数调用；空桩函数体实装并移除 `#[allow]`），
均落在本波次未提交的 `tui.rs` 工作区内；
`tests/`、`tools/`、`src/agent/` 零改动。工作树其余改动（`.dev-team/evidence/tui-polish/**` 的 E2E 重跑产物、
`src/stores/providers.ts` 的前端持久化改动、`review-wave39-*.json` 等）均为其他并行任务产物，非本 Agent 所写。

## 2. 门禁命令与真实退出码

> 退出码取自 `set -o pipefail` 下管道首段 cargo 的真实 `$?`。

### 2.1 A11 验收（契约 §5）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_usage_wave39_delta 2>&1 | tail -12
running 4 tests
test a11_other_events_yield_none ... ok
test a11_completed_yields_none ... ok
test a11_missing_fields_default_zero ... ok
test a11_output_end_yields_usage ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
EXIT=0
```

红相基线对照（实现前同命令）：`FAILED. 0 passed; 4 failed`，四条均为
`panicked at tui.rs:1296: not implemented`，`EXIT=101` —— 与 Test Agent 红相收据一致。

### 2.2 A1–A10 不回退

```
$ cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -18
test a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns ... ok
test a7_metrics_row_formats_compact_numbers_and_ratios ... ok
test a6_composer_geometry_is_input_row_plus_metrics_row ... ok
test a5_metrics_row_carries_up_down_cache_and_context ... ok
test a1_only_the_header_rule_keeps_the_hairline_glyph ... ok
test a4_metrics_row_left_shows_workspace_label ... ok
test a8_metrics_row_renders_unknown_placeholders ... ok
test a10_degenerate_widths_and_overlay_mode_degrade_without_panic ... ok
test a2_composer_shortcut_hints_are_gone ... ok
test a3_metrics_row_never_renders_status_words ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT=0
```

### 2.3 编译门禁

```
$ RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Checking pony-agent-core v0.1.127 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `dev` profile [unoptimized] target(s) in 1.24s
EXIT=0
```

### 2.4 回归基线（逐项一致）

| 套件 | 基线（wave-39 绿相收据 / 本 delta 前） | 本 delta 后 | Exit | 比对 |
|---|---|---|---|---|
| `tui_acceptance` | 34 passed / 1 failed（`test_matching_commands_filter`） | 34 passed / 1 failed（同一用例） | 101（同基线） | 一致，无新增失败 |
| `wave38-tui-workspace-contract` | 15 passed / 0 failed | 15 passed / 0 failed | 0 | 一致 |
| `tui_render_layout` | 3 passed / 5 failed（同五条陈旧用例） | 3 passed / 5 failed（同五条） | 101（同基线） | 一致，无新增失败 |

关键输出：

```
$ cargo test -p pony-agent-core --test tui_acceptance 2>&1 | tail -6
failures:
    test_matching_commands_filter
test result: FAILED. 34 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
EXIT=101

$ cargo test -p pony-agent-core --test wave38-tui-workspace-contract 2>&1 | tail -6
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
EXIT=0

$ cargo test -p pony-agent-core --test tui_render_layout 2>&1 | tail -10
failures:
    cjk_wrap_never_overflows_content_width
    conversation_hierarchy_and_streaming_render
    input_caret_tracks_cursor_position
    layout_invariants_hold_on_standard_terminal
    panel_renders_inside_screen_and_keeps_footer
test result: FAILED. 3 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out
EXIT=101
```

抓取日志（会话内临时件）：`/tmp/w39d_gate1.log`、`/tmp/w39d_gate2.log`、`/tmp/w39d_gate3.log`、
`/tmp/w39d_gate4a.log`、`/tmp/w39d_gate4b.log`、`/tmp/w39d_gate4c.log`。

## 3. 变更后的发射口径（行为说明）

| 事件名 | usage 发射 | StreamCompleted |
|---|---|---|
| `turn:output_end`，任一 token 字段 Some | 1 次 `StreamUsage`（缺失字段补 0） | 1 次 |
| `turn:output_end`，四字段全 None | 0 次 | 1 次 |
| `turn:completed`（无论载荷） | **0 次**（R8' 根因修复点） | 1 次 |
| `turn:delta` / `turn:failed` / `turn:tool` | 0 次 | 0 次 |

`resolve_context_window_tokens` / `apply_turn_usage` / `begin_turn_usage` 实现维持契约 §1 R3 不变，
本轮未触碰。

## 4. 未决风险

1. A11 只覆盖纯函数口径，未覆盖「新旧轮交叠」时序的端到端双计复现（需真实 TUI E2E，属验收方 V 阶段）；
   本 delta 已从发射源头（`turn:completed` 不再发 usage）消除二次 apply 路径。
2. `tui_acceptance` 的 `test_matching_commands_filter` 与 `tui_render_layout` 5 条为既有基线漂移，
   按契约 §4 Non-Goals/§5 裁决不修，未改断言。
3. 未发现契约与冻结断言冲突；测试零改动。
