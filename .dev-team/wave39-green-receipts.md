# wave-39 绿相收据（L2-E · Executor）

- 角色：Executor（实现者；测试冻结只读，未改动任何测试断言）
- 任务板：`task-3`（W39-E Composer 收边 + 底部指标行 + usage 接线）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §2（R4–R8）
- 代码基线：`316245b`（红相测试冻结）/ 空桩基线 `ba530bd`
- 环境：rustc 1.91.1 / cargo 1.91.1 / Linux
- 结论：**A1–A10 全绿 10 passed（Exit 0）；`-D warnings` 编译门禁 Exit 0；三项回归与基线逐条一致、无新增失败**

## 1. 改动清单（唯一文件：`crates/pony-agent-core/src/bin/tui.rs`）

写域审计：`git status --porcelain` 中 `crates/pony-agent-core/src/bin/tui.rs` 为唯一由本 Agent 修改的已跟踪文件；
`tests/`、`tools/`、`src/agent/` 零改动。工作树另有 `src/stores/providers.ts`（前端持久化去抖）改动，
其 mtime 晚于本次编辑且属并行前端任务，**非本 Agent 所写，未触碰**。

| 条款 | 改动 | 新文件行号 |
|---|---|---|
| R4 | 删除色带上方整行 `"▁"` 分隔行（原 HEAD 坐标 `tui.rs:2451-2465` 的 `if bottom_top > area.y { ... "▁".repeat(band_w) ... }`）；header 下方 `hairline()`（`y=1`）原样保留 | 删除处位于 Composer 段（现 2564–2604 区间） |
| R5 | `input_h = visual_lines.clamp(1, 5)`；`bottom_h = input_h + 2`；`input_rect.y = bottom_top + 1`；`chat_h = bottom_top - chat_top`（去掉 `-1`） | 2299 / 2300 / 2580 / 2304 |
| R6 | 删除底部 hint 五条串 + 右下角状态短语整块渲染（原 `tui.rs:2602-2652` 的 `hint`/`footer_line`/`footer_rect`）；`status_word()`（正文流式活动行使用）与 `compact_status()` 保留 | 删除处位于 Composer 段 |
| R7 | 新增指标行渲染：`metrics_line()`（整宽 SURFACE；`width > RAIL+1+right_w` → 左路径 `ellipsize_width` + 右簇右对齐；否则只渲染右簇；再否则截断到 `width`）；`workspace_label()`（`display()`，`$HOME`→`~`，`USERPROFILE` 兜底）；`fmt_token_count()`（<1000 原样 / <1e6 一位小数 `k` 且 `.0` 省略 / 否则 `M`；999_999→`1M` 防 `1000k`） | 1937（metrics_line）/ 1909（workspace_label）/ 1888（fmt_token_count）/ 2588（metrics_rect 接线） |
| R8-a | `TuiTurnSink::emit` 的 `"turn:completed" \| "turn:output_end"` 分支：四 token 字段任一 `Some` → 先发 `UiEvent::StreamUsage`（缺省 0）再发 `StreamCompleted` | 1265–1284 |
| R8-b | 主循环 `UiEvent::StreamUsage` 实装为 `state.apply_turn_usage(...)`（删除 `unimplemented!()` 占位） | 3768 |
| R8-c | 发送新消息前 `state.begin_turn_usage(); state.context_window_tokens = resolve_context_window_tokens(&state);` | 3653–3655 |
| R8-d | `begin_turn_usage` 置 `None`；`apply_turn_usage` 幂等覆盖（先扣旧回执再加新值、`last_context_tokens=total`、`turn_usage=Some(new)`） | 364 / 370 |
| R8-e | `resolve_context_window_tokens`：`ProviderRegistryStore::new().load_view()` → `resolve_active_provider_id` 命中 provider → 模型 `id`/`model` 命中 `state.model` → `capabilities.context_window_tokens`；否则 `1_000_000` | 668 |
| 清理 | 删除 Protocol 为过 `-D warnings` 加的 `#[allow(dead_code)]`（`turn_usage` 字段、`begin_turn_usage`）与 `#[allow(dead_code, unused_variables)]`（`apply_turn_usage`、`resolve_context_window_tokens`） | 266 / 363 / 369 / 667 附近 |

## 2. 门禁命令与真实退出码

> 退出码取自 `set -o pipefail` 下的 `$?`（即管道首段 cargo 的真实退出码）。

### 2.1 验收门禁（A1–A10）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -30
running 10 tests
test a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns ... ok
test a7_metrics_row_formats_compact_numbers_and_ratios ... ok
test a1_only_the_header_rule_keeps_the_hairline_glyph ... ok
test a6_composer_geometry_is_input_row_plus_metrics_row ... ok
test a5_metrics_row_carries_up_down_cache_and_context ... ok
test a10_degenerate_widths_and_overlay_mode_degrade_without_panic ... ok
test a4_metrics_row_left_shows_workspace_label ... ok
test a8_metrics_row_renders_unknown_placeholders ... ok
test a3_metrics_row_never_renders_status_words ... ok
test a2_composer_shortcut_hints_are_gone ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT=0
```

红相基线对照：实现前同命令 `FAILED. 0 passed; 10 failed`，`EXIT=101`。

### 2.2 编译门禁（无新增 warning）

```
$ cd /home/dm/pony-agent && RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Checking pony-agent-core v0.1.127 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `dev` profile [unoptimized] target(s) in 0.43s
EXIT=0
```

### 2.3 回归门禁（实现前同命令采基线，实现后逐项复跑）

| 套件 | 基线（实现前） | 实现后 | Exit | 比对结论 |
|---|---|---|---|---|
| `tui_acceptance` | 34 passed / 1 failed（`test_matching_commands_filter`） | 34 passed / 1 failed（同一用例） | 101（基线同） | **一致，无新增失败** |
| `wave38-tui-workspace-contract` | 15 passed / 0 failed | 15 passed / 0 failed | 0 | **一致，无回退** |
| `tui_render_layout` | 3 passed / 5 failed（`cjk_wrap_never_overflows_content_width`、`conversation_hierarchy_and_streaming_render`、`input_caret_tracks_cursor_position`、`layout_invariants_hold_on_standard_terminal`、`panel_renders_inside_screen_and_keeps_footer`） | 3 passed / 5 failed（同五条） | 101（基线同） | **一致，无新增失败** |

命令原文与关键输出：

```
$ cargo test -p pony-agent-core --test tui_acceptance 2>&1 | tail -6
failures:
    test_matching_commands_filter
test result: FAILED. 34 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
EXIT=101   # 与基线一致（契约口径：基线 34 passed / 1 failed=test_matching_commands_filter）

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
EXIT=101   # 与基线 3 passed / 5 failed 完全一致，未新增失败
```

抓取日志（会话内临时件）：`/tmp/w39_gate1.log`、`/tmp/w39_gate2.log`、`/tmp/w39_gate3a.log`、
`/tmp/w39_gate3b.log`、`/tmp/w39_gate3c.log`。

## 3. 真实渲染抽查（110×30，`state.set_input("hello")`，TestBackend）

从 `TUI_SNAPSHOT=1` 的缓冲 dump 还原（宽字符占位格保留原样）：

```
00|   pony  ·  pony-agent-core                     …                       gemini-3.8-flash   |
01|   ▁▁▁▁…（整行内容宽 104 列，仅此一条 ▁ 行）                                              |
02| （空）
03| （正文起点 / 空态留白）
...
27| （整行空白 = 色带顶部内边距）
28|   › hello
29|     ~/pony-agent/crates/pony-agent-core          ↑0 ↓0 · 缓存 — · 上下文 —
```

- y=1 是唯一 `"▁"` 行（R4）✓；y=27 全空白、y=28 提示符、y=29 指标行（R5/R6）✓；
- 指标行左起 `content.x+RAIL=5` 列为 `~`，右簇顶到内容列最后一列 `x=106`（R7）✓；
- 沟槽列 0/1/W-2/W-1 全程空白（A10 断言）✓。

## 4. 未决风险 / 说明

1. `resolve_context_window_tokens` 未被验收测试覆盖（A9 直接注入字段），其行为按契约 §1 R3 实现：
   注册表为空 / provider 或模型未命中 / capability 缺失均回退 `1_000_000`；`Some(0)` 按字面返回 0（指标行显示 `—`）。
2. `TuiTurnSink` 的 `"turn:completed"` 与 `"turn:output_end"` 可能先后各带一次同值载荷 → `apply_turn_usage`
   幂等覆盖，不会翻倍（A9 已断言幂等语义）。
3. `tui_render_layout` 的 5 条陈旧失败该套件自身断言旧布局（footer 状态词 `就绪`、`─` 发丝线字形等），
   与本波次契约冲突，按契约 §4 Non-Goals 不修、不下调断言；基线已记录在案。
4. 本 Agent 只读冻结测试；未发现契约与断言冲突。
