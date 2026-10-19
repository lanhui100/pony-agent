# wave-39 v3 红相收据（L2-T · Test Agent）

- 角色：独立 Test Agent（只写测试/审计工具；未碰 `tui.rs`、未改 A11 冻结文件、未放宽断言）
- 任务板：`task-8`（W39-E2 v3 红相迁移：几何/间距/更弱/无值隐藏）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §6 v3（R9/R10/R11/R12）
- 实现基线：HEAD `64a089f`（wave-39 v2 已闭合，`tui.rs` 干净未改）；v3 实现尚待 Executor
- 结论：**红相成立 —— 编译 Exit 0，运行 Exit 101，7 passed / 5 failed（A5/A6/A8/A12/A13 断言失败，0 panic）；checks.py 已适配 v3 语义，`py_compile` / `bash -n` Exit 0**

## 1. 交付物（写域内）

| 交付 | 路径 | 说明 |
|---|---|---|
| 冻结测试迁移 | `crates/pony-agent-core/tests/tui_dialog_wave39.rs`（877 行，`git diff` +297/−40） | A5/A6/A8 改写、A10 适配、新增 A12/A13；A1–A4/A7/A9 未改语义 |
| 审计脚本适配 | `tools/tui-audit/checks.py`（未跟踪） | 空态 footer / composer 边界 / 中途与失败轮 footer |
| 本收据 | `.dev-team/wave39-v3-red-receipts.md` | — |

写域审计（`git status --porcelain`，排除 E2E 证据目录）：

```
 M crates/pony-agent-core/tests/tui_dialog_wave39.rs   # 本轮唯一业务外编辑（契约演进授权）
 M .dev-team/contract-matrix-tui-dialog-wave39.md      # Lead 写入的 v3 契约，非本 Agent
 M src/components/chat/WorkspaceComposer.vue           # 并行前端任务
 M src/stores/providers.ts / tests/providers.store.spec.ts  # 并行前端任务
?? tools/                                             # Test Agent 工具目录（未跟踪）
```

`crates/pony-agent-core/src/bin/tui.rs` **零改动**（git 工作树无该文件修改；HEAD 即 v2 实现）。
A11 冻结文件 `tests/tui_usage_wave39_delta.rs` 未触碰；顺带复跑确认 R8' 已落地：**4 passed / Exit 0**。

## 2. 门禁命令与真实退出码

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 --no-run
    Finished `test` profile [unoptimized] target(s) in 3.56s
  Executable tests/tui_dialog_wave39.rs (target/debug/deps/tui_dialog_wave39-bb5f1391a69d031e)
EXIT_NO_RUN=0

$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -40
test result: FAILED. 7 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT_TEST=101        # 无管道真实退出码（PIPESTATUS[0]）

$ python3 -m py_compile tools/tui-audit/checks.py     → EXIT_PYCOMPILE=0
$ bash -n tools/tui-audit/run_audit.sh                → EXIT=0
```

完整日志：`/tmp/w39v3-red-final.log`（会话内临时件，失败原因已全部摘录于 §3）。

## 3. 红相失败分布（5 failed / 7 passed，单因：当前实现仍是 v2 布局）

| 断言 | 测试（行号） | 红相原因（真实 panic 摘要） | 契约 |
|---|---|---|---|
| A5 | `a5_metrics_row_carries_up_down_cache_and_context`（L334） | `无值状态不得渲染指标段 "↑" … "↑0 ↓0 · 缓存 — · 上下文 —"` | R11 |
| A6 | `a6_composer_geometry_is_band_padding_input_spacing_and_metrics`（L394） | `y=25 必须是色带顶部内边距（SURFACE）… 实际 Rgb(18,16,14)`（v2 色带从 y=27 起） | R9 |
| A8 | `a8_metrics_row_hides_segments_without_values`（L528） | `全零状态不得出现指标段/占位符 "↑" … "↑0 ↓0 · 缓存 — · 上下文 —"` | R11 |
| A12 | `a12_metrics_row_is_ghost_on_canvas_and_spacing_row_is_blank_canvas`（L751） | `指标行第 3 列背景必须是 CANVAS（非色带 SURFACE）… 实际 Rgb(27,25,22)` | R9/R10 |
| A13 | `a13_metrics_segments_hide_independently`（L808） | `window=0 时上下文段必须隐藏："… ↑500 ↓0 · 缓存 20% · 上下文 — "` | R11 |

通过（7）：A1（单发丝线）、A2（无 hint）、A3（末行无状态词）、A4（workspace 标签）、A7（1.2k/345/50%/25%）、A9（幂等累加）、A10（60/80 退化 + 浮层收起，已按 R11 注入有值状态）。

### 3.1 迁移要点（不放宽语义）

- **A5**：注入 1200/345/600/50k/200k 断言 `↑/↓/缓存/上下文` 全在且右端顶到内容列最后一列；新增「默认全零 → 右簇为空（无 ↑/↓/缓存/上下文/—），workspace 仍在」。
- **A6**：v3 几何三段证据 —— `y=24` CANVAS（正文）、`y=25` SURFACE 空白（bottom_top=25）、提示符唯一且落在 `26..=27`、`y=28` 空白且 CANVAS（间距行）、`y=29` 最后一行 CANVAS 且有 workspace、沟槽列空白、输入行右边界留白。
- **A8**：三用例 —— 全零 → 仅 workspace；`in=0` + window>0 → 仅 `上下文 25%`，不得出现 缓存/↑/↓/—；`window=0` + `last_ctx>0` → 不出现 上下文；`in>0` → ↑/↓/缓存正常。
- **A12**：指标行逐格断言 `bg == CANVAS`、非空白字形 `fg == Rgb(104,98,90)`（`theme` 私有，按字面值冻结）；间距行 28 逐格空白且 `bg == CANVAS`。
- **A13**：三段组合（仅 in>0 / 仅 window>0 / in=0 且 out>0）+ `assert_no_orphan_separator()`（不得出现 `· ·`、行尾 `·`、首个段前孤立 `·`）。
- **A10**：仅按 R11 注入有值状态（否则右簇为空、↑/↓ 断言失去意义），其余断言（不 panic、沟槽、右边界、浮层收起）原样保留。

## 4. `tools/tui-audit/checks.py` 改动点（仅 v3 直接相关，断言不放宽）

| 位置 | 改动 |
|---|---|
| `metrics_segments()`（L100）/ `metrics_row_index()`（L105） | 新增「已渲染段」判定；`metrics_row_index` 改以任一段存在为锚（空态无段 → None） |
| `bottom_chrome_start()`（L274）+ `composer_rows()`（L292） | 输入区下界改为「间距行 + 指标行」整体排除（v3 指标行移出色带、上方多 1 行 CANVAS 间距），`input_text()` 不再可能吸收间距/指标行 |
| `empty.footer_workspace_only`（L683） | 空态 footer：必须有 workspace（"pony-agent"）、**不得有任何 metrics 段**、不得有 `—` 占位符 |
| `streaming.mid_status_writing` / `mid_not_finished`（L1051/L1069） | 中途轮无 usage → footer 只可能带 `上下文`（窗口在发送前解析）；断言改为「active 行写入中 + footer 含 上下文 + 无 完成 状态词」 |
| `error.footer_metrics`（L1115） | 失败轮无 usage → footer 仅 `上下文`；断言改为「含 上下文 + 无 失败 状态词」 |
| 保持不动 | `conversation.footer_metrics` / `streaming.final_complete` / `scrolled.metrics_footer_*` 仍用 `is_metrics_line()`（完整四段，回合落定后有值）；其余 header/panel/命令表/输入编辑检查零改动 |

脚本自测（防假绿，`/tmp/w39v3-checks-selftest.py`，Exit 0）：
对合成 v3 截屏（色带 25-27 / 间距 28 / 指标行 29）验证 —— 空态 footer=workspace-only、`bottom_chrome_start==28`、`composer_rows` 只含输入行、单行 `input_text==""`、多行 `"line1\nline2"`、中途轮 context-only footer 通过；**负向对照**：v2 的 `缓存 — · 上下文 —` footer 被 v3 空态契约拒绝。

## 5. 交接给 Executor（v3 实现）的绿相门禁

1. `cargo test -p pony-agent-core --test tui_dialog_wave39` → **12 passed / Exit 0**（A1–A13，A11 在独立文件已 4/4）。
2. R9：`input_h=(visual_lines+1).clamp(2,6)`、`band_h=input_h+1`、`bottom_h=input_h+3`、110×30 单行 `bottom_top=25`、间距行 28 CANVAS、指标行 29 CANVAS。
3. R10：新增 `theme::GHOST=Rgb(104,98,90)`，指标行整行 GHOST、bg CANVAS，不用 `Modifier::DIM`。
4. R11：右簇按有值拼装（↑/↓ 仅 in||out>0；缓存仅 in>0；上下文仅 window>0；无 `—`、无孤立 `·`）。
5. 实现仅限 `tui.rs`；不得改本测试、A11 测试与 checks.py；完成后由 Test Agent 复跑 E2E（含空态 workspace-only 新断言）。
