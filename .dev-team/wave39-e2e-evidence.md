# wave-39 独立验收凭证（L2-V · Test Agent）

- 角色：独立 Test Agent（只写验证产物；未改业务代码、未改已冻结断言、未放宽 checks.py）
- 任务板：`task-4`（W39-V 独立验收）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §2 R4–R8 / §3 A1–A10
- 被测二进制：`target/debug/pony` · `pony 0.1.127 · linux-x86_64` · sha256 `a1ab84bf330ce2371c46174d8be6c744f60fc0ff6cd63c05cec4049708f8bd98` · mtime `2026-10-19T20:00:27+08:00`
- 结论：**契约断言全绿（A1–A10 10/10、E2E 中 58 条 wave-39 相关检查全 PASS）；
  E2E 门禁 Exit 1（FAIL），4 条失败全部为「8 命令 vs 7 命令冻结期望」的既有基线漂移，经 pre-wave-39 二进制定量复现，归因不属于本波次，建议 Lead 按已知基线裁决。**

## 1. 交付 1：独立复跑冻结测试（真实 Exit Code）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -20
test a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns ... ok
test a7_metrics_row_formats_compact_numbers_and_ratios ... ok
test a1_only_the_header_rule_keeps_the_hairline_glyph ... ok
test a5_metrics_row_carries_up_down_cache_and_context ... ok
test a6_composer_geometry_is_input_row_plus_metrics_row ... ok
test a10_degenerate_widths_and_overlay_mode_degrade_without_panic ... ok
test a4_metrics_row_left_shows_workspace_label ... ok
test a8_metrics_row_renders_unknown_placeholders ... ok
test a2_composer_shortcut_hints_are_gone ... ok
test a3_metrics_row_never_renders_status_words ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
EXIT_WAVE39=0          # 无管道时的真实退出码
```

红相基线（本 Agent 上一交付）：`0 passed / 10 failed / Exit 101` → 绿相 `10 passed / Exit 0`，无假绿。

## 2. 交付 2：回归基线比对（逐项失败用例名）

| 套件 | 基线 | 本次实测 | Exit | 失败用例名 | 结论 |
|---|---|---|---|---|---|
| `tui_acceptance` | 34 passed / 1 failed | 34 passed / 1 failed | 101 | `test_matching_commands_filter` | 与基线**完全一致**，无新增 |
| `wave38-tui-workspace-contract` | 15 passed / 0 failed | 15 passed / 0 failed | 0 | — | **一致**，无回退 |
| `tui_render_layout` | 3 passed / 5 failed（五条陈旧用例） | 3 passed / 5 failed | 101 | `cjk_wrap_never_overflows_content_width`、`conversation_hierarchy_and_streaming_render`、`input_caret_tracks_cursor_position`、`layout_invariants_hold_on_standard_terminal`、`panel_renders_inside_screen_and_keeps_footer` | 与基线**同五条**，无新增、未修复（Non-Goals） |

命令（均真实执行）：

```
$ cargo test -p pony-agent-core --test tui_acceptance        → EXIT_ACCEPTANCE=101（34/1）
$ cargo test -p pony-agent-core --test wave38-tui-workspace-contract → EXIT_W38=0（15/0）
$ cargo test -p pony-agent-core --test tui_render_layout      → EXIT_LAYOUT=101（3/5）
```

## 3. 交付 3：真实 TUI E2E（Tier-1 物理凭证）

### 3.1 构建与被测二进制身份

```
$ cd /home/dm/pony-agent && cargo build -p pony-agent-core --bin pony
   Compiling pony-agent-core v0.1.127 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `dev` profile [unoptimized] target(s) in 58.04s
EXIT_BUILD=0
$ sha256sum target/debug/pony
a1ab84bf330ce2371c46174d8be6c744f60fc0ff6cd63c05cec4049708f8bd98  target/debug/pony
```

### 3.2 审计执行（真实退出码 = 1 → 门禁 FAIL）

```
$ cd /home/dm/pony-agent && tools/tui-audit/run_audit.sh --phase after --bin target/debug/pony
== version=pony 0.1.127 · linux-x86_64 sha256=a1ab84bf330ce237… mtime=2026-10-19T20:00:27+08:00
...
{"report_json": ".../after/report.json", "pass": 365, "fail": 4, "failed_scenarios": ["empty", "slash-menu"]}
SUMMARY: 365 pass / 4 fail across 18 scenarios
failed_scenarios=empty,slash-menu
EXIT_AUDIT=1
```

- **E2E 门禁 Exit Code：`1`（FAIL）**，非 0；原因见 §3.3 / §4（既有基线漂移，非 wave-39）。
- 报告：`.dev-team/evidence/tui-polish/after/report.json`、`report.md`（baseline 对比已内嵌）。
- 与 pre-refactor 基线（`pony 0.1.125`）对比：`354 checks / 48 fail / 15 场景失败` → `369 checks / 4 fail / 2 场景失败`；FAIL→PASS 39 项（含 wave-39 的 `empty.footer_metrics`、`conversation.footer_metrics`、`error.footer_metrics`、`streaming.*`、`main.separator_*` 等新口径）。

### 3.3 4 条失败断言（如实记录，未放宽 checks.py）

| 场景 | 断言 | 证据（真实屏幕文本） |
|---|---|---|
| empty | `empty.command_desc_column` | `cols={'/resume':16,'/provider':16,'/models':16,'/clear':None,…} missing=['/clear']`；屏幕空态命令表为 `/resume /provider /models /thinking`（`take(4)` 截断） |
| slash-menu | `slash.all_commands_listed` | `missing=['/quit']`；菜单 7 行止于 `/help` |
| slash-menu | `slash.each_command_once` | `duplicate/missing commands=['/quit'] menu_rows=7` |
| slash-menu | `slash.command_desc_column` | `'/quit': None`（第 8 个命令被 7 行上限截掉） |

屏幕原文（slash-menu 菜单区，y=22..30）：

```
   ──────────────────────────────────────────────
   ▏ /resume    选择并恢复历史会话
     /provider  管理与切换模型提供商
     /models    切换当前活跃模型
     /thinking  设置思考强度 (Reasoning Effort)
     /clear     清空当前消息上下文
     /cost      查看当前模型与网关配额信息
     /help      查看所有支持的快捷指令
   ──────────────────────────────────────────────
```

**判定**：4 条失败均为「命令面在 pre-wave-39 已扩到 8 条（新增 `/thinking`）而 checks.py 仍以 7 命令表＋`take(4)`/`min(7)` 上限为期望」的既有漂移；wave-39 执行器 diff 对 `AVAILABLE_COMMANDS` / `ascii_logo` / `min(7)` / `take(4)` **零改动**（`git diff` 命中 0 行）。定量归因见 §4。

### 3.4 wave-39 契约检查（E2E 内逐条全 PASS，58/58）

- 指标行 footer：`empty.footer_metrics`、`conversation.footer_metrics`、`error.footer_metrics`、`scrolled.metrics_footer_before|after`、`streaming.mid_status_writing|mid_not_finished|final_complete|final_not_writing` 全 PASS；
- 单发丝线：`*.separator_single_header_rule` + `*.separator_header_rule_geometry` 全 PASS（`rule_rows=[1]`，start 与 width 随 120/60/160 列正确随动 3/114、3/54、20/120）；
- 无 panic：全部 capture `no_panic` 检查 PASS（58/58，含 ctrl-c/非 TTY/CLI）；
- 全量 capture 静态复核（脚本 `/tmp/w39v-verify-captures.py`，EXIT=0）：**32 个 capture → 24 个正文 TUI 截屏最后一行=指标行（含 ↑/↓/缓存/上下文）、4 个浮层截屏 composer 收起、4 个退出后 shell/CLI 截屏豁免；所有 TUI 截屏 `▁` 整行恰好 1 条；`Enter 发送`/`Esc 中断`/`Shift+Enter`/`就绪`/`已上翻` 残留 = 0；panic = 0**。

指标行实拍（`*.txt` 最后一行原文）：

| 场景/截屏 | 最后一行（指标行） |
|---|---|
| empty/main (120×34) | `/home/dm/pony-agent … ↑0 ↓0 · 缓存 — · 上下文 —` |
| conversation/main | `/home/dm/pony-agent … ↑32 ↓64 · 缓存 0% · 上下文 0%` |
| narrow/conversation (60×20) | `/home/dm/pony-agent    ↑32 ↓64 · 缓存 0% · 上下文 0%` |
| wide/conversation (160×40) | `/home/dm/pony-agent … ↑32 ↓64 · 缓存 0% · 上下文 0%` |
| streaming/mid | `/home/dm/pony-agent … ↑0 ↓0 · 缓存 — · 上下文 0%` |
| error/main | `/home/dm/pony-agent … ↑0 ↓0 · 缓存 — · 上下文 0%` |
| scrolled/after | `/home/dm/pony-agent … ↑32 ↓64 · 缓存 0% · 上下文 0%` |

> `缓存 —` / `上下文 —` = 契约 R7/A8 未知态占位符（会话未产生 usage / 窗口未知），非缺陷；HOME 被审计脚本隔离为临时目录，故标签显示绝对路径而非 `~`，符合 `workspace_label()` 语义。

### 3.5 物理凭证（截图 / ANSI / 文本，已落盘）

| 类型 | 路径 | 说明 |
|---|---|---|
| PNG（2280×1428 px，2x 渲染） | `.dev-team/evidence/tui-polish/after/empty/main.png`（144,125 B） | 空态全屏：仅 1 条 header 发丝线、无 hint/状态词、底部指标行；见文末附图 |
| ANSI（SGR 原文） | `.dev-team/evidence/tui-polish/after/empty/main.ansi`（3,557 B） | 同一帧的可回放色带凭证 |
| PNG（窄屏） | `.dev-team/evidence/tui-polish/after/narrow/conversation.png` | 60×20 指标行含真实 token `↑32 ↓64 · 缓存 0%` |
| 文本 | `.dev-team/evidence/tui-polish/after/*/ *.txt` | 18 场景 32 份 capture（含 mid/final/after/exit 等） |
| 报告 | `.dev-team/evidence/tui-polish/after/report.json` / `report.md` | 365/369 通过，逐断言证据可复核 |

![wave-39 empty 空态：单发丝线 + 底部指标行](.dev-team/evidence/tui-polish/after/empty/main.png)

## 4. 归因实验：同一冻结 checks 对 pre-wave-39 二进制复跑

为排除「4 条失败由 R4–R8 引入」的可能，用 `git archive ba530bd`（Protocol 空桩提交 = Executor 改动前）导出隔离快照到 `/tmp/w39-head`，同源码构建二进制并跑同一份（未放宽）checks：

```
$ git archive ba530bd | tar -x -C /tmp/w39-head     # 仓库工作树零改动（未 stash、未 checkout）
$ CARGO_TARGET_DIR=/tmp/w39-head/target cargo build -p pony-agent-core --bin pony   # Exit 0
  binary sha256=637757b961187749e5803cdbd99971478e226cf685ac71806fa8a4dcca23e7f5
$ /tmp/w39-head/tools/tui-audit/run_audit.sh --phase after --bin /tmp/w39-head/target/debug/pony --only empty,slash-menu
RESULT scenario=empty pass=11 fail=4
RESULT scenario=slash-menu pass=11 fail=3
EXIT_PRE_AUDIT=1
```

| 断言 | pre-wave-39（ba530bd 构建） | wave-39 after（本次） | 归因结论 |
|---|---|---|---|
| `slash.all_commands_listed`（/quit 缺） | **FAIL** | **FAIL** | 既有漂移，与本波次无关 |
| `slash.each_command_once`（/quit 缺） | **FAIL** | **FAIL** | 既有漂移 |
| `slash.command_desc_column`（/quit 缺） | **FAIL** | **FAIL** | 既有漂移 |
| `empty.command_desc_column`（/clear 缺） | **FAIL** | **FAIL** | 既有漂移 |
| `empty.footer_metrics` | **FAIL**（旧 hint+就绪行） | **PASS** | wave-39 R6/R7 **修复项** |
| `main.separator_single_header_rule` | **FAIL**（`rule_rows=[1,28]` 双发丝线） | **PASS**（`rule_rows=[1]`） | wave-39 R4 **修复项** |

结论：4 条失败在 wave-39 之前即已存在（命令面 `/thinking` 引入于 `6429be7`，早于波次起点 `ba530bd`）；wave-39 相关断言在 pre 版本红、在 after 版本绿 —— **本波次契约无回归，修复生效**。归因凭证已固化到 `.dev-team/evidence/tui-polish/wave39-attribution/`（pre 二进制的 bin-info + empty/slash-menu 的 assertions/main.txt）。

> pre 快照中 `empty/main.header` 另有一条失败，原因是我把快照导出到 `/tmp/w39-head`，header 工作区 basename 变 `w39-head`，属隔离目录命名导致的 harness 伪影，与业务无关。

## 5. 交付 4：工作树冒烟与进程清理

```
$ cd /home/dm/pony-agent && git status --porcelain | wc -l
158
$ git status --porcelain | grep -vE "tui-polish/after/"
 M crates/pony-agent-core/src/bin/tui.rs     # Executor R4–R8（被测对象，非本 Agent 所改）
 M src/stores/providers.ts                   # 并行前端任务改动，Executor 收据已披露，非本波次
?? .dev-team/evidence/tui-polish/wave39-attribution/  # 本轮归因凭证（pre-wave-39 复跑）
?? .dev-team/wave39-e2e-evidence.md          # 本凭证
?? .dev-team/review-wave39-{a,b,p}.json      # 其他 Agent 评审产物
?? .dev-team/wave39-green-receipts.md        # Executor 绿相收据
?? tools/                                    # 本波次 Test Agent 工具目录（未跟踪）
```

- 本次审计仅刷新 `.dev-team/evidence/tui-polish/after/**`（149 条已跟踪证据的再生成）＋新增
  `.dev-team/wave39-e2e-evidence.md`、`.dev-team/evidence/tui-polish/wave39-attribution/`；**未触碰 tui.rs / tests/ / checks.py**。
- 进程与 tmux 残留（审计脚本 trap cleanup 后复核）：

```
$ pgrep -af "mock_llm.py|run_audit.sh|/tmp/w39-head/target/debug/pony"   → 无输出
$ pgrep -af "/target/debug/pony"                                          → 无输出
$ for s in /tmp/tmux-1000/tui-audit-*; do tmux -L "$s" ls; done           → 无一存活（no server running）
$ tmux -L tui-audit-3046229 ls                                            → no server running（本次 socket 已清理）
```

## 6. 裁决建议（交 Lead）

1. **wave-39 契约面：PASS** —— A1–A10 10/10；E2E 中 58 条 wave-39 检查全 PASS；单发丝线/指标行/hint 清除/无 panic 全部由真实 tmux + mock LLM 抓屏证实。
2. **E2E 门禁：FAIL（Exit 1）** —— 仅因 4 条「既有命令面漂移」断言，且经 pre-wave-39 二进制 1:1 复现，不属于本波次回归。按纪律**未放宽 checks.py**、未改冻结断言。
3. 建议按「已知基线」裁决（与 `tui_render_layout` 5 条陈旧失败同级），并另开工单处理 `/thinking` 命令面与 checks.py 7 命令期望的漂移（候选方案：菜单上限与空态引导命令表随命令总数自适应，或 checks.py 期望改为从命令表推导 —— 均超出 wave-39 写域）。
4. 遗留风险：
   - `run_audit.sh` 中对 `完成|Turn completed`（conversation/sessions/streaming/narrow/wide/scrolled）与 `已上翻`（scrolled）的 `wait_for` 在 R6 后已永不命中，每场景空转 8–15s（仅耗时，不影响断言；`scrolled.after_shows_earlier_rows`/`offset_moved_up` 仍 PASS，证明 PageUp 翻页正常）。建议后续一并清理，本轮未改。
   - `tui_render_layout` 5 条陈旧失败维持基线，未修复（契约 §4 Non-Goals）。
   - 归因快照 `/tmp/w39-head`（1.1G，含独立 target）为临时验证件，可随时删除；已固化的小体积凭证在 `.dev-team/evidence/tui-polish/wave39-attribution/`。
