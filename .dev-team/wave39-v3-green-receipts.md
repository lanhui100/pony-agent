# wave-39 v3 绿相收据（L2-E · Executor · R9/R10/R11 + 收尾清理）

- 角色：Executor（只改 `crates/pony-agent-core/src/bin/tui.rs`；测试冻结，未改任何断言）
- 任务板：`task-9`（W39-E3 v3 实施）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §6（R9/R10/R11/R12）
- 基线：v3 红相 `c233c34`；A12/A13 用例缺陷由 Test Agent 经 Lead 授权修正
  （见 `.dev-team/wave39-v3-testfix.md`，Executor 未碰测试）
- 环境：rustc/cargo 1.91.1，Linux
- 结论：**12 passed（A1–A13）/ 4 passed（A11）/ `-D warnings` check Exit 0；回归与基线逐项一致；no-op 后置补色已删除**

## 1. 改动清单（唯一文件：`crates/pony-agent-core/src/bin/tui.rs`）

| 条款 | 改动 | 最终行号 |
|---|---|---|
| R9 | `input_h = (visual_lines + 1).clamp(2, 6)`；`band_h = input_h + 1`；`bottom_h = input_h + 3`；`chat_h = bottom_top - chat_top` 不变 | 2321–2324 |
| R9 | 色带 Rect 高度改 `band_h`（1 行顶部内边距 + input_h）；色带下方 1 行间距行不画字符（CANVAS 保持） | 2589–2596 |
| R9 | 指标行 Rect 移到色带外：`y = bottom_top + band_h + 1`，`bg = CANVAS`；110×30 单行 → bottom_top=25 / 色带 25..=27 / 间距 28 / 指标 29 | 2611–2622 |
| R10 | `theme::GHOST = Color::Rgb(104, 98, 90)`（比 FAINT 暗一档）；指标行整行 fg=GHOST，不加 `Modifier::DIM` | 1466–1467 / 1948–2001 |
| R11 | `metrics_line` 右簇分段「有值才显示」：`↑in ↓out` 仅 `in>0\|\|out>0`；`缓存 pct%` 仅 `in>0`；`上下文 pct%` 仅 `window>0`；段间 `" · "` 连接；全无值 → 右簇为空（仅 workspace）；无 `—`、无孤立 `·` | 1948–2001 |
| R11 | 左侧 workspace 仍从 `content.x + RAIL` 起、右簇右对齐至 `content.x + content.width`；空间不足优先保留右簇并截断左路径，绝不越内容列 | 1978–2001 |
| 浮动层 | 浮层态（mode != Normal）仍整体不渲染色带/间距行/指标行 | 2590–2623 |

## 2. 门禁命令与真实退出码（收尾清理后复跑）

> 退出码取自 `set -o pipefail` 下管道首段 cargo 的真实 `$?`。

### 2.1 A1–A13 验收

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -20
running 12 tests
test a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns ... ok
test a12_metrics_row_is_ghost_on_canvas_and_spacing_row_is_blank_canvas ... ok
test a7_metrics_row_formats_compact_numbers_and_ratios ... ok
test a6_composer_geometry_is_band_padding_input_spacing_and_metrics ... ok
test a5_metrics_row_carries_up_down_cache_and_context ... ok
test a1_only_the_header_rule_keeps_the_hairline_glyph ... ok
test a10_degenerate_widths_and_overlay_mode_degrade_without_panic ... ok
test a4_metrics_row_left_shows_workspace_label ... ok
test a13_metrics_segments_hide_independently ... ok
test a8_metrics_row_hides_segments_without_values ... ok
test a3_metrics_row_never_renders_status_words ... ok
test a2_composer_shortcut_hints_are_gone ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT=0
```

### 2.2 A11（R8'，独立冻结文件不回退）

```
$ cargo test -p pony-agent-core --test tui_usage_wave39_delta 2>&1 | tail -8
running 4 tests
test a11_completed_yields_none ... ok
test a11_missing_fields_default_zero ... ok
test a11_output_end_yields_usage ... ok
test a11_other_events_yield_none ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
EXIT=0
```

### 2.3 编译门禁

```
$ RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Checking pony-agent-core v0.1.127 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `dev` profile [unoptimized] target(s) in 0.94s
EXIT=0
```

### 2.4 回归基线（逐项一致，未新增失败）

| 套件 | 基线 | v3 后 | Exit | 结论 |
|---|---|---|---|---|
| `tui_acceptance` | 34 passed / 1 failed（`test_matching_commands_filter`） | 34 passed / 1 failed（同一用例） | 101（同基线） | 一致 |
| `wave38-tui-workspace-contract` | 15 passed / 0 failed | 15 passed / 0 failed | 0 | 一致 |
| `tui_render_layout` | 3 passed / 5 failed（同五条陈旧用例） | 3 passed / 5 failed（同五条） | 101（同基线） | 一致 |

## 3. v3 收尾清理（L3-B 复审 PASS 后）

### 3.1 删除内容（原 `tui.rs:2622-2625`，共 4 行）

```rust
        // ratatui 0.29 在宽字符（缓存/上下文）右侧占位格上会 reset()，把背景打回 Reset；
        // v3 契约要求指标行整行 CANVAS 背景，故渲染后仅补回背景色（不动字形与前景）。
        f.buffer_mut()
            .set_style(metrics_rect, Style::default().bg(theme::CANVAS));
```

### 3.2 删除理由

L3-B 复审实测该后置补色是 **no-op（frame-buffer diff=0）**：`Paragraph::render` 已对整区
`set_style(area, style)`，指标行真实背景本来就是 CANVAS；宽字符续格 Reset 只发生在
**TestBackend 的 `Buffer::diff` 输出侧**（`to_skip` 跳过续格），不是真实渲染背景缺陷。
原注释的因果链不成立，保留会误导后人，故连同语句一起删除。

### 3.3 保留项

`Paragraph::new(metrics_line(...)).style(Style::default().bg(theme::CANVAS))` 正常渲染段（现
`tui.rs:2617-2621`）保持不动；未改任何其他逻辑、常量或测试；`grep buffer_mut` 在 `tui.rs` 中已为 0 命中。

### 3.4 清理后门禁（真实退出码）

| 门禁 | 结果 | Exit |
|---|---|---|
| `cargo test -p pony-agent-core --test tui_dialog_wave39` | 12 passed / 0 failed | **0** |
| `cargo test -p pony-agent-core --test tui_usage_wave39_delta` | 4 passed / 0 failed | **0** |
| `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony` | Finished，无 warning | **0** |

抓取日志（会话内临时件）：`/tmp/w39v3c_g1.log`、`/tmp/w39v3c_g2.log`、`/tmp/w39v3c_g3.log`、
`/tmp/w39v3c_r1.log`、`/tmp/w39v3c_r2.log`、`/tmp/w39v3c_r3.log`。

## 4. 说明与未决风险

1. A12/A13 的绿相依赖 Test Agent 对**用例自身缺陷**的修正（Lead 授权，见
   `.dev-team/wave39-v3-testfix.md`）：Executor 未改测试；修正仅跳过宽字符续格断言与纠正
   孤立分隔符检测逻辑，未放宽 `GHOST`/`CANVAS` 字面契约。
2. `tui_acceptance` 的 `test_matching_commands_filter` 与 `tui_render_layout` 五条为既有基线
   漂移，按契约 Non-Goals/§5 裁决不修。
3. 本波次只改 `tui.rs`；工作树中 `src/components/chat/WorkspaceComposer.vue`、
   `src/stores/providers.ts`、`tests/providers.store.spec.ts` 等为并行前端任务产物，非本 Agent 所写。
