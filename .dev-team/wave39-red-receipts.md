# wave-39 红相收据（L2-T · Test Agent）

- 角色：独立 Test Agent（测试分权制：只写测试，不改业务实现）
- 任务板：`task-2`（W39-T 红相冻结验收测试）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §3（A1–A10）；§2 R4–R7 为断言依据
- 空桩基线：`.dev-team/wave39-protocol-receipt.md`（commit `ba530bd`）
- 结论：**红相成立 —— 编译通过（Exit 0），运行失败 Exit 101，0 passed / 10 failed**

## 1. 交付物（写域内）

| 交付 | 路径 | 性质 |
|---|---|---|
| 验收测试（新文件） | `crates/pony-agent-core/tests/tui_dialog_wave39.rs`（620 行） | A1–A10，ratatui `TestBackend` 真实渲染断言 |
| 审计脚本适配 | `tools/tui-audit/checks.py`（未被 git 跟踪） | 单发丝线 / 指标行 footer / input_text 边界 |
| 本收据 | `.dev-team/wave39-red-receipts.md` | — |

写域审计：`git diff --stat` 仅 `.dev-team/state.json`（团队看板，非本 Agent）；`crates/pony-agent-core/src/bin/tui.rs` **零改动**（未出现在 diff 中）；tests/ 仅新增本文件。

## 2. 门禁命令与真实退出码

### 2.1 编译门禁（红相必须是「编译通过 + 断言失败」）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 --no-run
   Compiling pony-agent-core v0.1.127 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `test` profile [unoptimized] target(s) in 0.45s
  Executable tests/tui_dialog_wave39.rs (target/debug/deps/tui_dialog_wave39-bb5f1391a69d031e)
EXIT_NO_RUN=0
```

### 2.2 红相门禁（契约指定命令）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -40
...
test result: FAILED. 0 passed; 10 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
error: test failed, to rerun pass `-p pony-agent-core --test tui_dialog_wave39`
EXIT_TEST=101        # 无管道时的真实退出码（PIPESTATUS[0]）
```

完整日志：`/tmp/w39-red-run.log`（会话内临时件，本收据已摘录全部失败原因）。

### 2.3 脚本侧门禁

```
$ python3 -m py_compile tools/tui-audit/checks.py     # EXIT=0
$ python3 /tmp/w39-checks-selftest.py                 # EXIT=0  （合成契约截屏自测，见 §5）
SELFTEST_OK
```

## 3. 红相失败分布（10/10 失败，无一通过）

失败性质分布：**断言失败 9 组 + 未实现 `unimplemented!()` panic 1 组**（无编译错误、无环境错误）。

| # | 测试 | 红相原因（真实 panic 摘要） | 契约条款 |
|---|---|---|---|
| A1 | `a1_only_the_header_rule_keeps_the_hairline_glyph` | `left: [1, 24] / right: [1]` —— 全屏 `"▁"` 行有 2 条（header 之外色带分隔线仍在） | R4 |
| A2 | `a2_composer_shortcut_hints_are_gone` | `[empty] 正常态仍渲染已废弃的快捷键提示 "Enter 发送"` | R6 |
| A3 | `a3_metrics_row_never_renders_status_words` | `[ready] 最后一行（指标行）不得再出现状态词 "就绪"` —— 最后一行仍是状态行 | R6/R7 |
| A4 | `a4_metrics_row_left_shows_workspace_label` | 最后一行左起为 hint 文本，`~/pony-agent` 标签缺失 | R7 |
| A5 | `a5_metrics_row_carries_up_down_cache_and_context` | `指标行缺少 "↑"` —— 最后一行无指标簇 | R7 |
| A6 | `a6_composer_geometry_is_input_row_plus_metrics_row` | 提示符行实际 `[27]`，期望 `[28]`（旧几何 `bottom_h=5`） | R5 |
| A7 | `a7_metrics_row_formats_compact_numbers_and_ratios` | `1200 输入 token 必须格式化为 "1.2k"` —— 指标行未实现 | R7 |
| A8 | `a8_metrics_row_renders_unknown_placeholders` | `in=0 时缓存占用率必须显示占位符 "缓存 —"` —— 指标行未实现 | R7 |
| A9 | `a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns` | `panicked at src/bin/tui.rs:376: not implemented`（`apply_turn_usage` 空桩） | R3 |
| A10 | `a10_degenerate_widths_and_overlay_mode_degrade_without_panic` | `60×20 最后一行必须仍是指标行（↑/↓）` —— 窄屏最后一行仍是 hint+状态 | R5/R7 |

> A10 在红相失败于「窄屏指标行缺失」，浮层态断言顺延未执行；绿相时两条路径都必须通过。

## 4. 测试设计要点（防假测试）

- **真实渲染**：`#[path = "../src/bin/tui.rs"] mod tui;` + `Terminal::new(TestBackend::new(w,h))` + `terminal.draw(|f| tui::ui(f, state))`，断言直接读取字符缓冲，不 mock 布局。
- **视觉字符串还原**：缓冲区里 CJK 宽字符右侧有一格占位空格（`"缓存 50%"` 会呈现为 `缓 存 50%`），因此 `row_text()` 按 `disp_width()` 跳过占位 cell 还原终端视觉串，避免「假绿/假红」；几何断言仍按 cell 列取值（`row_cells()` / `buf[(x,y)]`）。
- **几何按契约复算**：`content_geometry()` 复刻 `GUTTER=3 / MAX_CONTENT_WIDTH=120`，110×30 → `content.x=3, width=104`；断言提示符列、内容列右边界、沟槽列（0/1/W-2/W-1）。
- **AAA 结构**：每个测试 Arrange / Act / Assert 三段注释齐全；A9 为纯状态机断言（3 段共 12 条断言），A2/A3 覆盖 5/7 种运行态分支。
- **未知态独立**：A8 拆两次渲染分别锁 `in=0 → 缓存 —`、`window=0 → 上下文 —`，不互相掩盖。
- 无 `sleep`、无网络、无临时文件、无跨测试共享状态（每个测试自建 `TuiState`）。

## 5. `tools/tui-audit/checks.py` 改动点（仅 wave-39 直接相关，其余检查保留）

| 改动 | 位置 | 内容 |
|---|---|---|
| 指标行识别原语 | `checks.py:88-101` | 新增 `METRICS_TOKENS = ("↑","↓","缓存","上下文")`、`is_metrics_line()`、`metrics_row_index()` |
| `separator_checks()` 契约更新 | `checks.py:453-475` | 由「>= 2 条 `▁` 且 header/composer 同字形同宽」改为**恰好 1 条**（`separator_single_header_rule`）+ header 行位/起点/宽度（`separator_header_rule_geometry`） |
| `composer_rows()` 边界适配 | `checks.py:266-281` | 重建输入区时以 `metrics_row_index()` 为下界（指标行现在是最后一行，绝不计入输入文本）；无指标行的旧截屏回退到最后一行 |
| `input_text()` 文档化适配 | `checks.py:283-293` | 注明指标行不属于输入缓冲（行为由 `composer_rows` 保证） |
| footer 断言 → 指标行契约 | `empty.footer_metrics` / `conversation.footer_metrics` / `error.footer_metrics` / `scrolled.metrics_footer_before|after` | 就绪 / 完成 / 失败 / 已上翻 的「状态短语」断言改为「footer 是指标行 + 旧状态短语不得出现」 |
| 流式状态迁移 | `streaming.mid_status_writing` / `mid_not_finished` / `final_complete` / `final_not_writing` | `写入中` 改为正文活动行契约；完成/未完成以「指标行存在 + 终结行文本」判定，不再读 footer 状态词 |
| 注释同步 | `composer_boundary()` docstring | 说明 R4 已删除 composer 顶部 `▁`，prompt 行为唯一锚点 |

**未改动**：header、面板几何、focus 模式、命令表、输入编辑、Ctrl-C、non-TTY、CLI 等全部既有检查保持不变（仅上述 footer/分隔线/输入重建三处与 wave-39 契约强相关）。

自测凭证（防脚本假绿）：对**合成 wave-39 契约截屏**（110×30，header 单发丝线 + 空输入 + 指标行 `↑1.2k ↓345 · 缓存 50% · 上下文 25%`，左标签 `~/pony-agent`）运行改写后的断言，`separator_*` 2/2 通过、`composer_rows` 精确排除指标行、`input_text==""`；同一脚本对**遗留双发丝线截屏**判定 `separator_single_header_rule=False`（负向对照生效）。对既有 18 个真实截屏目录（拷贝到 `/tmp`，不污染仓库证据）跑 `checks.py check` 全部无异常退出。

## 6. 基线声明（不计入本波次失败，未修复）

- `tui_render_layout`：3 passed / 5 failed（陈旧用例，契约 §4 Non-Goals 明确不修）
- `tui_acceptance`：34 passed / 1 failed
- `wave38`：15 passed
- 本波次测试文件为独立 `--test` 目标，不与上述套件共享状态或编译单元。

## 7. 交接给 Executor（task-3）的绿相门禁

1. `cargo test -p pony-agent-core --test tui_dialog_wave39` 必须 `10 passed; 0 failed`（Exit 0）。
2. `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony` 必须 Exit 0（测试文件不在 bin 编译单元，但不得为通过而改测试）。
3. 指标行文案严格按 R7：`↑{in} ↓{out} · 缓存 {cache%} · 上下文 {ctx%}`；`缓存 —` / `上下文 —` 用 U+2014；数字格式 `<1000` 原样、`<1e6` 一位小数 `k`（`.0` 省略）。
4. 本文件与 A1–A10 断言**冻结**，执行器只读；任何断言不通过 = 实现缺陷，不得改测试放宽。
