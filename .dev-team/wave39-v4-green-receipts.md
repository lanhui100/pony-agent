# wave-39 v4 绿相收据（L2-E5 · Executor · R13 间距减半）

- 角色：Executor（只改 `crates/pony-agent-core/src/bin/tui.rs`；测试冻结，未改任何断言）
- 任务板：`task-11`（W39-E5）—— 开工时任务板 **未找到 task-11**（列表仅 task-1..task-10，均 completed），
  已按 Lead 指令实现；完成后再次 `team_task_get("task-11")` 仍为 not found，无法 claim/complete，已在回报中说明
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §7（R13/R14）
- 基线：v3 已提交（`e7b771b`），v4 红相 `bf1bd9f`（10 passed / 2 failed = A6/A12）
- 环境：rustc/cargo 1.91.1，Linux
- 结论：**12 passed（A1–A13）/ 4 passed（A11）/ `-D warnings` check Exit 0；回归与基线逐项一致**

## 1. 改动清单（唯一文件：`crates/pony-agent-core/src/bin/tui.rs`，+18/−6）

| 条款 | 改动 | 最终行号 |
|---|---|---|
| R13 | `bottom_h = input_h + 2`（原 `+3`，删除独立 CANVAS 间距行）；`band_h = input_h + 1`、`input_h`、`chat_h` 均不变 | 2324 |
| R13 | 指标行紧贴色带下方：`metrics_y = bottom_top + band_h`（原 `+ band_h + 1`），仍是终端最后一行 | 2625 |
| R13 | 删除 v3「色带下方隔 1 行间距」注释，改为 v4 语义 | 2621–2622 |
| R14/A6 | 指标行 `Paragraph` 样式补 `fg = GHOST`：指标行所覆盖的内容列内空白格（如 `content.x` 处 RAIL 空格）也算「整行 GHOST 前景」，满足 A6 对 `(content.x, 29)` 的 fg 断言 | 2631 |
| R14/A12 | 色带末行（呼吸行）底色按**色带 content 宽**铺 SURFACE（`x=content.x / width=content.width`）：它是「输入正文 → 指标行」之间唯一的 1 行视觉间距；左右沟槽列保持 CANVAS，指标行列宽不变 | 2601–2612 |

110×30 单行输入实测几何：`bottom_top=26`；色带 `26..=28`（26 顶部内边距 / 27 输入正文 / 28 呼吸空白 SURFACE）；
指标行 `29`（CANVAS + GHOST，紧贴色带）；输入正文行到指标行之间只隔 1 行空白（v3 为 2 行）。

## 2. 门禁命令与真实退出码

> 退出码取自 `set -o pipefail` 下管道首段 cargo 的真实 `$?`。

### 2.1 A1–A13 验收

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -20
running 12 tests
test a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns ... ok
test a12_metrics_row_is_ghost_on_canvas_and_breathing_row_is_band_surface ... ok
test a1_only_the_header_rule_keeps_the_hairline_glyph ... ok
test a6_composer_geometry_is_band_padding_input_spacing_and_metrics ... ok
test a7_metrics_row_formats_compact_numbers_and_ratios ... ok
test a5_metrics_row_carries_up_down_cache_and_context ... ok
test a10_degenerate_widths_and_overlay_mode_degrade_without_panic ... ok
test a4_metrics_row_left_shows_workspace_label ... ok
test a13_metrics_segments_hide_independently ... ok
test a8_metrics_row_hides_segments_without_values ... ok
test a2_composer_shortcut_hints_are_gone ... ok
test a3_metrics_row_never_renders_status_words ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT=0
```

红相基线对照（v4 实现前同命令）：`FAILED. 10 passed; 2 failed`（A6/A12），`EXIT=101`。

### 2.2 A11（R8'，独立冻结文件不回退）

```
$ cargo test -p pony-agent-core --test tui_usage_wave39_delta 2>&1 | tail -8
running 4 tests
test a11_output_end_yields_usage ... ok
test a11_completed_yields_none ... ok
test a11_missing_fields_default_zero ... ok
test a11_other_events_yield_none ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
EXIT=0
```

### 2.3 编译门禁

```
$ RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Checking pony-agent-core v0.1.127 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `dev` profile [unoptimized] target(s) in 0.79s
EXIT=0
```

### 2.4 回归基线（逐项一致，未新增失败）

| 套件 | 基线 | v4 后 | Exit | 结论 |
|---|---|---|---|---|
| `tui_acceptance` | 34 passed / 1 failed（`test_matching_commands_filter`） | 34 passed / 1 failed（同一用例） | 101（同基线） | 一致 |
| `wave38-tui-workspace-contract` | 15 passed / 0 failed | 15 passed / 0 failed | 0 | 一致 |
| `tui_render_layout` | 3 passed / 5 failed（同五条陈旧用例） | 3 passed / 5 failed（同五条） | 101（同基线） | 一致 |

抓取日志（会话内临时件）：`/tmp/w39v4_g1.log`、`/tmp/w39v4_g2.log`、`/tmp/w39v4_g3.log`、
`/tmp/w39v4_r1.log`、`/tmp/w39v4_r2.log`、`/tmp/w39v4_r3.log`。

## 3. 需要 Lead 知悉的两处「契约文字之外、冻结断言之内」的实现细节

1. **呼吸行宽度已按内容列收敛（Lead 裁决，Test Agent 同步收敛 A12）**：初版为过旧 A12
   （`for x in 0..buf.area.width { bg == SURFACE }`）曾把呼吸行铺满终端整行；Lead 裁决「呼吸行
   只按色带 content 宽铺 SURFACE，沟槽列保持 CANVAS」，Test Agent 已将 A12 改为「内容列逐格
   SURFACE + 沟槽列（0/1/W-2/W-1）行 28/29 CANVAS 且无字形」。当前实现：
   `Rect { x: content.x, y: bottom_top+band_h-1, width: content.width, height: 1 }`，与色带主体
   （26–27 行）及指标行同宽，沟槽列保持 CANVAS。
2. **指标行 `Paragraph` 样式加 `fg = GHOST`**：A6 要求 `(content.x, 29)` 的前景是 GHOST，而该格是
   指标行内 RAIL 空白（无字形）；仅给有字形的 Span 设 GHOST 无法满足，故把前景设到整区样式上。
   语义与 R10「指标行整行 fg=GHOST」一致，无副作用（空白格 fg 不可见）。

## 4. 未决风险

1. `task-11` 在任务板上不存在（本 Agent 无法 claim/complete）；请 Lead 确认看板同步或另建任务。
2. 呼吸行整行 SURFACE 对真实终端是 9 级 RGB 的极弱差异（SURFACE `#1B1916` vs CANVAS `#12100E`），
   E2E checks.py 只做文本断言，不受影响；但若后续视觉审计要求「色带左右边界完全一致」，按 §3.1 处理。
3. 本波次只改 `tui.rs`；工作树中前端文件（`WorkspaceComposer.vue`、`providers.ts` 等）为并行任务产物，
   非本 Agent 所写。

## 5. 收尾修正记录（task-11 流程：A12 收敛后）

Lead 裁决把呼吸行从「铺满终端整行」收敛为「色带 content 宽」，Test Agent 已于
2026-10-19 21:22:47 更新 `crates/pony-agent-core/tests/tui_dialog_wave39.rs`（A12：内容列逐格
`bg == SURFACE`；沟槽列 0/1/W-2/W-1 在 y=28/29 必须 `bg == CANVAS` 且无字形）。Executor 随后：

- 呼吸行填充改为 `Rect { x: content.x, y: bottom_top + band_h - 1, width: content.width, height: 1 }`
  （`tui.rs:2601–2612`），不再铺满终端整行；
- 指标行 `Paragraph` 样例保留 `.style(bg(theme::CANVAS).fg(theme::GHOST))`（`tui.rs:2630–2634`），
  空段格也有 GHOST 前景，满足 A6 `(content.x, 29)` 断言；
- 未改其他逻辑/常量/测试。

收敛后门禁（真实退出码）：

```
$ cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -20
running 12 tests
test a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns ... ok
test a12_metrics_row_is_ghost_on_canvas_and_breathing_row_is_band_surface ... ok
test a1_only_the_header_rule_keeps_the_hairline_glyph ... ok
test a6_composer_geometry_is_band_padding_input_spacing_and_metrics ... ok
test a7_metrics_row_formats_compact_numbers_and_ratios ... ok
test a5_metrics_row_carries_up_down_cache_and_context ... ok
test a10_degenerate_widths_and_overlay_mode_degrade_without_panic ... ok
test a4_metrics_row_left_shows_workspace_label ... ok
test a13_metrics_segments_hide_independently ... ok
test a8_metrics_row_hides_segments_without_values ... ok
test a2_composer_shortcut_hints_are_gone ... ok
test a3_metrics_row_never_renders_status_words ... ok
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT=0

$ cargo test -p pony-agent-core --test tui_usage_wave39_delta 2>&1 | tail -8
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
EXIT=0

$ RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Finished `dev` profile [unoptimized] target(s) in 0.34s
EXIT=0
```

回归（收敛后复跑，逐项与基线一致）：

| 套件 | 结果 | Exit | 基线比对 |
|---|---|---|---|
| `tui_acceptance` | 34 passed / 1 failed（`test_matching_commands_filter`） | 101 | 一致 |
| `wave38-tui-workspace-contract` | 15 passed / 0 failed | 0 | 一致 |
| `tui_render_layout` | 3 passed / 5 failed（同五条陈旧用例） | 101 | 一致 |

抓取日志：`/tmp/w39v4b_g1.log`、`/tmp/w39v4b_g2.log`、`/tmp/w39v4b_g3.log`、
`/tmp/w39v4b_r1.log`、`/tmp/w39v4b_r2.log`、`/tmp/w39v4b_r3.log`。
