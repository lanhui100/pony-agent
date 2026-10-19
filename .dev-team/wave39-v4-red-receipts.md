# wave-39 v4 红相收据（间距减半 · L2-T）

- 角色：独立 Test Agent（只写测试/审计工具，未碰 `tui.rs`）
- 任务板：`task-10`（W39-E4 v4 红相）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §7 v4（R13/R14）
- 实现基线：v3 已提交（工作树 `tui.rs` 干净），v4 实现尚待 Executor
- 结论：**红相成立 —— 编译 Exit 0，运行 Exit 101，10 passed / 2 failed（A6/A12 断言失败，0 panic）；checks.py 边界已按 v4 适配，`py_compile` Exit 0**

## 1. 交付物（写域内）

| 交付 | 路径 | 说明 |
|---|---|---|
| 冻结测试迁移 | `crates/pony-agent-core/tests/tui_dialog_wave39.rs`（`git diff --stat` +38 / −26） | A6/A12 迁移；A1–A5/A7–A11/A13 未改 |
| 审计脚本适配 | `tools/tui-audit/checks.py`（未跟踪） | `bottom_chrome_start()` 边界语义（L274） |
| 本收据 | `.dev-team/wave39-v4-red-receipts.md` | — |

写域审计（排除并行前端/E2E 目录）：仅 `crates/pony-agent-core/tests/tui_dialog_wave39.rs` 被改；`tui.rs` 零改动；A11 冻结文件零改动；`checks.py` 仅改边界函数与文档。

## 2. 门禁命令与真实退出码

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 --no-run
    Finished `test` profile [unoptimized] target(s) in 3.55s
EXIT_NO_RUN=0

$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -40
test result: FAILED. 10 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT_TEST=101        # 无管道真实退出码（PIPESTATUS[0]）

$ cargo test -p pony-agent-core --test tui_usage_wave39_delta   → 4 passed / EXIT_A11=0
$ python3 -m py_compile tools/tui-audit/checks.py               → EXIT_PYCOMPILE=0
$ bash -n tools/tui-audit/run_audit.sh                          → EXIT_BASH_N=0
```

完整日志：`/tmp/w39v4-red-run.log`。

## 3. 红相失败分布（2 failed / 10 passed，单因：当前实现仍是 v3 的「独立 CANVAS 间距行」）

| 断言 | 位置 | 失败原因（真实 panic 摘要） | 契约 |
|---|---|---|---|
| A6 | `a6_composer_geometry_is_band_padding_input_spacing_and_metrics`（L394） | `y=25 必须仍是正文画布 CANVAS（v4 色带不得从 25 行开始）实际 Rgb(27,25,22)` —— v3 `bottom_top=25` | R13 |
| A12 | `a12_metrics_row_is_ghost_on_canvas_and_breathing_row_is_band_surface`（L784） | `色带呼吸行第 0 列背景必须是 SURFACE（v4：间距行已删除，呼吸行属于色带），实际 Rgb(18,16,14)` —— v3 第 28 行是 CANVAS 间距行 | R13/R14 |

### 3.1 A6 迁移要点（语义收紧，未放宽）

- Assert 1：`y=25` 必须空白且 bg=CANVAS（正文画布），`y=26` 空白且 bg=SURFACE（`bottom_top=26` 的直接证据）；
- Assert 2：提示符唯一且位于 `26..=27`，落在内容列起点；
- Assert 3：`y=28` 为**色带内**呼吸行（空白 + bg=SURFACE），并断言 `prompt_y + 2 == 29` —— 输入正文行到指标行之间**只隔 1 行空白**（v3 为 2 行），即「间距减半」的机器可判定证据；
- Assert 4：`y=29` 指标行 bg=CANVAS + fg=GHOST + workspace 仍在；
- Assert 5/6：沟槽列空白、输入行右边界留白（原样）。

### 3.2 A12 迁移要点

- 指标行（y=29）非续格 `fg == GHOST(104,98,90)` / `bg == CANVAS(18,16,14)`，宽字符续格跳过逻辑**保留**（v3 test-fix 已记录 ratatui `Buffer::diff` 首帧 `to_skip` 语义）；
- y=28 由「间距行逐格 CANVAS」改为「色带呼吸行逐格空白 + bg=SURFACE」（宽字符无涉，逐格严格）。

## 4. `checks.py` 改动（仅边界语义）

- `bottom_chrome_start()`（L274）：删除「向上跨过一行空白」的 v3 逻辑；v4 中指标行上方的空白行是**色带内呼吸行（SURFACE）**，不属于 chrome，故边界 = 指标行本行（无指标段时回退最后一行）。`composer_rows()`（L288）继续过滤空白行，输入缓冲重建行为不变。
- 空态 footer 断言（`empty.footer_workspace_only`）与其它 metrics/footer 断言保持不变（v4 不改变指标行内容与位置=最后一行）。
- 自测（`/tmp/w39v4-checks-selftest.py`，Exit 0）：合成 v4 截屏（色带 26–28 / 指标 29）下 empty=workspace-only、`bottom_chrome_start==29`、单行 `input_text==""`、多行 `"line1\nline2"`、mid-turn context-only footer 全部通过。

## 5. 交接给 Executor（v4 实现）的绿相门禁

1. `cargo test -p pony-agent-core --test tui_dialog_wave39` → **12 passed / Exit 0**；A11 保持 4 passed。
2. R13：删除独立 CANVAS 间距行；`bottom_h = input_h + 2`（110×30 单行 `bottom_top=26`）；色带 `26..=28`（26 顶部内边距 / 27 输入正文 / 28 呼吸空白，均 SURFACE）；指标行 29 紧贴色带下方，CANVAS 背景 + GHOST 前景。
3. 实现仅限 `tui.rs`；不得改冻结测试、A11 测试与 checks.py；完成后由 Test Agent 复跑 E2E（checks.py 边界已就绪）。
