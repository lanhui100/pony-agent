# 红相收据 · TUI 思考消息可见 + 消息区底部间距（thinking-bottom-gap）

- 任务：`task-1`（S2，L2-T 独立验收测试 / 红相冻结）
- 契约真值源：`.dev-team/contract-matrix-thinking-bottom-gap.md`（C1–C12）
- 唯一验收测试文件：`crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs`
- SUT（只读，未改动）：`crates/pony-agent-core/src/bin/tui.rs`
- 执行环境：Linux（`/home/dm/pony-agent`），`cargo` 原生集成测试目标 `--test tui_thinking_bottom_gap`
- 冻结时刻：2026-11-06（本文件落盘后，测试文件对 Executor **只读**，禁止改断言放宽）

## 1. 命令原文与真实退出码

命令原文（逐字）：

```
cd /home/dm/pony-agent && cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test tui_thinking_bottom_gap --target-dir target-test 2>&1 | tail -60
```

执行方式：同一命令加 `set -o pipefail`（仅为了拿到 cargo 的真实退出码而不是 `tail` 的 0）：

```
cd /home/dm/pony-agent && set -o pipefail && cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test tui_thinking_bottom_gap --target-dir target-test 2>&1 | tail -60; echo "EXIT_CODE=$?"
```

**真实退出码：`EXIT_CODE=101`**（cargo test 失败态）。
**编译结果：编译成功**（日志中无 `error[E…]`、无 `warning: unused` 级阻断，仅正常构建输出）。
**测试统计：`test result: FAILED. 15 passed; 10 failed; 0 ignored; 0 measured; 0 filtered out`**

完整日志（辅助凭证，非入仓产物）：`/tmp/red-phase-thinking-bottom-gap.log`

## 2. 用例判定清单（A1–A11）

| 用例 | 对应契约 | 本次判定 | 红相归属（SUT panic / 断言失败） |
| :-- | :-- | :-- | :-- |
| A1 `a1_delta_payload_text_only` | C1/C2 | FAILED | SUT `todo!()` panic @ `tui.rs:4597` |
| A2 `a2_delta_payload_channel_isolation` | C1/C2/C3 | FAILED | SUT `todo!()` panic @ `tui.rs:4597` |
| A3 `a3_reasoning_chunks_merge_into_single_thinking_message` | C4 | FAILED | SUT `todo!()` panic @ `tui.rs:3671` |
| A4 `a4_thinking_and_assistant_stay_separate` | C4/C7 | FAILED | SUT `todo!()` panic @ `tui.rs:3671` |
| A5 `a5_reasoning_updates_runtime_mode_without_stealing_tool_state` | C5 | FAILED | SUT `todo!()` panic @ `tui.rs:3671` |
| A6 `a6_thinking_message_renders_with_faint_italic_and_glyph` | C6 | FAILED | SUT `todo!()` panic @ `tui.rs:6207`（Thinking 渲染分支） |
| A7 `a7_single_breathing_line_between_thinking_and_answer` | C6/C7 | FAILED | SUT `todo!()` panic @ `tui.rs:6207` |
| A8 `a8_bottom_gap_holds_when_auto_scrolled_to_bottom` | C8/C9 | FAILED | 断言失败（间距未实现：`last_chat_rect` 下缘 26 ≠ 间距行 25） |
| A9 `a9_streaming_status_line_keeps_bottom_gap` | C8/C10 | FAILED | 断言失败（间距行被「思考中」状态行占据） |
| A10 `a10_tiny_terminal_keeps_gap_without_panic` | C11 | FAILED | 断言失败（24×10 下消息区下缘 6 > prompt_row-2 = 5） |
| A11 `a11_existing_roles_render_unchanged_without_thinking_glyph` | C12 | **ok（回归守卫）** | 绿色符合预期：既有 4 类角色无思考字形泄漏 |

失败 10 / 通过 1（A11）。另 14 条通过项来自 `tui.rs` 自身随 `#[path]` 带入的 `#[cfg(test)]` 单元测试
（`tui::splash_gradient_tests::*` 9 条 + `tui::wave43_unit_tests::*` 5 条），证明**测试夹具能编译并真实运行**。

## 3. 关键失败输出 tail（≥15 行，原始 stdout）

```
---- a1_delta_payload_text_only stdout ----

thread 'a1_delta_payload_text_only' (1781385) panicked at crates/pony-agent-core/tests/../src/bin/tui.rs:4597:5:
not yet implemented: 契约 C1/C2/C3：turn:delta 双通道取载未实现

---- a3_reasoning_chunks_merge_into_single_thinking_message stdout ----

thread 'a3_reasoning_chunks_merge_into_single_thinking_message' (1781387) panicked at crates/pony-agent-core/tests/../src/bin/tui.rs:3671:9:
not yet implemented: 契约 C4/C5：思考增量入库未实现

---- a6_thinking_message_renders_with_faint_italic_and_glyph stdout ----

thread 'a6_thinking_message_renders_with_faint_italic_and_glyph' (1781390) panicked at crates/pony-agent-core/tests/../src/bin/tui.rs:6207:21:
not yet implemented: 契约 C6：思考块渲染未实现

---- a10_tiny_terminal_keeps_gap_without_panic stdout ----

thread 'a10_tiny_terminal_keeps_gap_without_panic' (1781383) panicked at crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs:559:5:
契约 C11：24×10 极小终端下消息区下缘（3 + 3 = 6）仍必须 ≤ prompt_row - 2 = 5，不得把间距行挤进 Composer；屏幕 = …

---- a8_bottom_gap_holds_when_auto_scrolled_to_bottom stdout ----

thread 'a8_bottom_gap_holds_when_auto_scrolled_to_bottom' (1781392) panicked at crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs:479:5:
assertion `left == right` failed: 契约 C8：消息区下缘（last_chat_rect.y=3 + height=23 = 26）必须正好停在间距行 y=25，否则最后一行正文会贴住 Composer 色带
  left: 26
 right: 25

---- a9_streaming_status_line_keeps_bottom_gap stdout ----

thread 'a9_streaming_status_line_keeps_bottom_gap' (1781393) panicked at crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs:522:5:
契约 C8/C10：流式状态行之下的间距行（y=25）必须全空白，实际 = "   · 思 考 中 …"；屏幕 = 
…
   · 思 考 中
   
   › 写 下 任 务 ， 或 输 入  / 查 看 命 令
   
     ~/pony-agent/crates/pony-agent-core

failures:
    a10_tiny_terminal_keeps_gap_without_panic
    a1_delta_payload_text_only
    a2_delta_payload_channel_isolation
    a3_reasoning_chunks_merge_into_single_thinking_message
    a4_thinking_and_assistant_stay_separate
    a5_reasoning_updates_runtime_mode_without_stealing_tool_state
    a6_thinking_message_renders_with_faint_italic_and_glyph
    a7_single_breathing_line_between_thinking_and_answer
    a8_bottom_gap_holds_when_auto_scrolled_to_bottom
    a9_streaming_status_line_keeps_bottom_gap

test result: FAILED. 15 passed; 10 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s

error: test failed, to rerun pass `--test tui_thinking_bottom_gap`
```

## 4. 红相归属说明（一句话）

**红相归属：全部 10 条失败均落在 SUT（`crates/pony-agent-core/src/bin/tui.rs`）侧——A1–A7 是 SUT 三处空桩
`todo!()`（`stream_delta_ui_payload` @4597、`TuiState::apply_stream_reasoning` @3671、`Thinking` 渲染分支 @6207）的原生 panic，
A8–A10 是 SUT 未实现 `CHAT_BOTTOM_GAP` 导致的真实断言失败；无一条源自测试夹具、编译错误或跳过用例。**

## 5. 红相纯净性核对

- **编译门禁**：`cargo test` 到达 `test result:` 汇总行（退出码 101 = 测试失败，非 101 之前的 cargo 编译错误），
  日志无 `error[E…]`；若存在编译错误，收据不会出现 `test result:` 行。
- **零假测试**：11 条用例全部含真实 `assert_eq!/assert!`（无 `is_ok()` 空跑、无 `#[ignore]`、无 `panic::catch_unwind` 吞异常）。
- **夹具自证**：A2 的 DSML 样本先以 `tui::sanitize_model_protocol_text` 断言「净化后为空且 `incomplete=true`」，
  确保样本本身成立；A6/A8/A9/A10 先断言 `last_chat_rect` 已写入、`max_scroll_offset > 0`、屏幕唯一 `›` 行，
  再断言契约条款，避免「夹具没跑起来也算红」。
- **夹具修正记录（红相前，非放宽断言）**：首轮执行时 A9 的 CJK 断言用了 `all_text().contains("思考中")`，
  而宽字符右侧占位 cell 是空白 ⇒ 屏幕串为 `"思 考 中"`，属**测试夹具缺陷**而非 SUT 问题；
  已改为与既有 `tui_render_layout.rs` 同范式的 `squeeze()`（去空白后匹配）后再执行，断言强度不变。
- **确定性**：渲染类断言使用 `screen_until_settled` 的**显式条件轮询**（条件 `scroll_offset >= max_scroll_offset`，
  上限 400 帧），无 `sleep`、无随机种子、无系统时间依赖（`turn_started_at=None` ⇒ 耗时标签为空）。
- **写域纪律**：仅写本收据与测试文件两个路径；未修改 `tui.rs`；**未执行任何 git 写操作**（add/commit/stash/checkout 均无）。

## 6. 执行侧后续（给 Executor）

绿相只需让上述 11 条全绿，且**不得**修改本测试文件；实现面：
C1/C2/C3（`stream_delta_ui_payload`）→ C4/C5（`apply_stream_reasoning`）→ C6/C7（Thinking 渲染分支）→
C8/C9/C10/C11（`chat_h` 扣减 `CHAT_BOTTOM_GAP`，含极小终端饱和语义）→ C12（既有角色不回归，含 `tui_render_layout` 全绿）。

---

# 追加红相 · A12 / A13 / A14（2026-11-06 23:3x；L3-B 审查 F1 + 契约修订 R1/R3/C4a）

## 7. 追加范围与纪律

- **只追加**：新增内容从测试文件第 609 行起（`// ── A12：思考块前驱侧间距 …` 节），
  A1–A11 块边界仍为**第 1–606 行逐字不变**（`grep -c '^#\[test\]'` = 14：A1–A14）。
- 新增 3 条用例 + 2 个辅助（`screen_sequence` / `assert_sentinel_gap`，均为真实断言载体）。
- 未修改 `crates/pony-agent-core/src/bin/tui.rs`；未做任何 git 写操作。

## 8. 命令与真实退出码

```
cd /home/dm/pony-agent && cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test tui_thinking_bottom_gap --target-dir target-test > /tmp/red-phase-tbg-a12-a14.log 2>&1; echo "EXIT_CODE=$?"
```
（等价口径：`… 2>&1 | tail -60`；退出码取 cargo 本体，见 `set -o pipefail` 或直接重定向。）

**真实退出码：`EXIT_CODE=101`**
**统计：`test result: FAILED. 31 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out`**
完整日志：`/tmp/red-phase-tbg-a12-a14.log`

## 9. 逐条判定明细

| 用例 | 对应契约 | 判定 | 明细 |
| :-- | :-- | :-- | :-- |
| A12 `a12_thinking_keeps_one_breath_against_previous_non_user_blocks` | C6 修订 R1（F1/BLOCKING） | **PASS（已绿）** | 非测试缺陷：Executor 已先行落地 F1 修复（`tui.rs` mtime `2026-11-06 23:25:55`；当前实现在块首 `lines.push(blank())` 且排除 `User` 前驱）。杀伤力另由 §10 变异实验物理证明。 |
| A13 `a13_delta_payload_preserves_whitespace_only_text` | C3 修订 R3 | **FAILED** | `left: None / right: Some("\n\n")` —— SUT `stream_delta_ui_payload` 用 `cleaned.trim().is_empty()`（`tui.rs:4637`）判空，把段落分隔符 `"\n\n"` 误丢；单空格增量同样被吞。 |
| A14 `a14_thinking_block_is_bounded_by_turn_started_reset` | C4a 修订（思考按轮分块） | **FAILED** | `assertion left == right failed: 实际 2 条，角色序列 = [User, Thinking]` —— `TuiState::begin_turn_usage()`（`tui.rs:2970`）只清 usage 快照，未置思考块轮边界，第二轮思考续写进上一轮块。 |

原始失败输出：

```
---- a13_delta_payload_preserves_whitespace_only_text stdout ----

thread 'a13_delta_payload_preserves_whitespace_only_text' (2179714) panicked at crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs:737:5:
assertion `left == right` failed: 契约 C3(修订 R3)：正文通道纯空白增量 "\n\n"（段落分隔）必须原样保留，用 trim() 判空会丢弃空行导致段落丢失，实际 = DeltaUiPayload { text: None, reasoning: None }
  left: None
 right: Some("\n\n")

---- a14_thinking_block_is_bounded_by_turn_started_reset stdout ----

thread 'a14_thinking_block_is_bounded_by_turn_started_reset' (2179715) panicked at crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs:781:5:
assertion `left == right` failed: 契约 C4a：turn:started 之后到达的思考增量必须**新开** Thinking 块（User + 2×Thinking = 3 条），实际 2 条，角色序列 = [User, Thinking]
  left: 2
 right: 3

failures:
    a13_delta_payload_preserves_whitespace_only_text
    a14_thinking_block_is_bounded_by_turn_started_reset

test result: FAILED. 31 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

## 10. A12 杀伤力证明（隔离变异实验，仓库零改动）

A12 在本轮**执行时已绿**（SUT 已被 Executor 修好），为免「空转断言」存疑，做一次**变异实验**证明其可杀伤缺陷：

1. 把 `crates/pony-agent-core` 复制到仓库外的 `/tmp/tbg-mutation/pony-agent-core`（SUT 本体零改动）。
2. 仅把思考块前驱呼吸 hunk **回退为修复前形态**：
   ```rust
   if ctx.prev_role.is_some() && !matches!(ctx.prev_role, Some(MessageRole::Thinking)) {
       push_blank(&mut lines);
   }
   ```
3. 在该隔离副本内跑 A12：
   ```
   cd /tmp/tbg-mutation/pony-agent-core && set -o pipefail && cargo test --test tui_thinking_bottom_gap a12 --target-dir /tmp/tbg-mutation/target; echo "EXIT_CODE=$?"
   ```
   → **`EXIT_CODE=101`，`test result: FAILED. 0 passed; 1 failed`**，
   `契约 C6(R1)：Assistant → Thinking 前驱侧必须恰好 1 行呼吸（行差 2），不得 0 行粘连：
   哨兵行差必须为 2（= 中间 1 行空白），实际 before=3 → after=4 行差 1；left: 1 / right: 2`
   —— 与 L3-B 审查实测的「Assistant→Thinking = 0 行粘连」逐字吻合。
4. 证据日志：`/tmp/tbg-mutation-a12-falsifiability.log`；变异副本已删除（过程产物不入仓）。

**结论：A12 不是空转断言——F1 缺陷存在时它必红，修复后它转绿。**

## 11. 追加红相归属（一句话）

**A13/A14 的失败均落在 SUT（`tui.rs:4637` 的 `trim()` 判空语义、`tui.rs:2970` 的 `begin_turn_usage()` 缺思考块轮边界），
是真实断言失败（`left/right` 值明确），非测试夹具、非编译错误、非 skip；A12 当前为绿属 SUT 已修复，其红证据由 §10 变异实验提供。**

---

# 追加红相 · 底部间距改 2 行（A8/A9/A10，契约 C8 修订 R5）

## 12. 追加范围、时序事实与命令

**范围（严格限定）**：只把 A8/A9/A10 的**间距断言**从 1 行改为 2 行；`A1–A7`、`A11–A14` 断言一字未改
（同一次运行中它们全部 `ok`）。具体改动：

- A8：`tui::CHAT_BOTTOM_GAP == 2`；间距行 = `[prompt-3, prompt-2]` 逐行 `trim().is_empty()`；
  `last_chat_rect.y + height == prompt-3`；`row_text(prompt-4).trim()` 非空。
  另新增**底色证据**：两行间距 `bg == theme::CANVAS (18,16,14)`，色带上缘 `prompt-1` 的 `bg == theme::SURFACE (27,25,22)`
  ⇒ 证明 2 行间距确实落在色带之外，而非被色带吞掉。
- A9：两行 `[prompt-3, prompt-2]` 全空白；`last_chat_rect` 下缘 `== prompt-3`；「思考中」在 `prompt-4`。
- A10：`last_chat_rect.y + height <= prompt-3`（= `prompt - (1 + CHAT_BOTTOM_GAP)`，C11 修订口径）。

**时序事实（如实登记）**：我在 23:45:3x 读取源码时 `CHAT_BOTTOM_GAP` 仍为 **1**（`tui.rs:4915`），
Executor 于 **23:45:50** 并发把常量改为 2（`tui.rs` mtime）；我 23:46 执行时已全绿。
故本轮**未能捕获直接红相**（SUT 先于测试落地），红证据由 §13 的两个变异实验提供。

命令与真实退出码（当时已绿）：

```
cd /home/dm/pony-agent && cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test tui_thinking_bottom_gap --target-dir target-test > /tmp/red-phase-tbg-gap2.log 2>&1; echo "EXIT_CODE=$?"
```

→ **`EXIT_CODE=0`**，`test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`。

## 13. 杀伤力证明（隔离变异实验，仓库零改动）

副本：`crates/pony-agent-core` → `/tmp/tbg-mut2/pony-agent-core`（仓库外；SUT 本体零改动），
在副本内各注入一处最小变异后跑同一测试目标。证据日志：`/tmp/tbg-gap2-mutant-a.log`、`/tmp/tbg-gap2-mutant-b.log`
（变异副本已删除，过程产物不入仓）。

### 变异 A：常量回退到 1（`CHAT_BOTTOM_GAP = 1`）

```
cd /tmp/tbg-mut2/pony-agent-core && cargo test --test tui_thinking_bottom_gap --target-dir /tmp/tbg-mut2/target
→ EXIT_CODE=101，test result: FAILED. 32 passed; 3 failed
```

- A8：`assertion left == right failed: 契约 C8(修订 R5)：…常量必须为 2…实际 = 1`（`left: 1 / right: 2`）
- A9：`契约 C8(修订 R5)/C10：流式状态行之下的间距行 y=24（共 1 行：[24, 25]）必须全空白，实际 = "   · 思 考 中 …"`（**几何**失败，未依赖常量断言）
- A10：`契约 C11(修订 R5)：24×10 … 消息区下缘（3 + 2 = 5）仍必须 ≤ prompt_row - 3 = 4`（**几何**失败）

### 变异 B：常量保持 2，但几何只扣 1 行（`chat_h … .saturating_sub(1)`）

```
→ EXIT_CODE=101，test result: FAILED. 32 passed; 3 failed
```

- A8：`契约 C8(修订 R5)：间距行 y=24（共 2 行：[24, 25]）必须全空白…实际 = "     第 39问 ： …"`（**常量断言通过**，纯几何断言击穿）
- A9：`契约 C8(修订 R5)/C10：… 间距行 y=24 … 实际 = "   · 思 考 中 …"`
- A10：`契约 C11(修订 R5)：消息区下缘（3 + 2 = 5）仍必须 ≤ prompt_row - 3 = 4`

**结论：A8/A9/A10 的 2 行口径不是空转——常量层与几何层各自被独立变异击穿；绿相只能由「常量=2 且几何扣满 2 行」同时满足。**

## 14. 本轮归属与纪律

- 变异实验证明的三处失败点全部位于 SUT 的间距实现（常量 / `chat_h` 扣减），非测试夹具、非编译错误、非 skip。
- 本轮只改 A8/A9/A10 的间距断言与两处 doc 注释；`A1–A7`、`A11–A14` 未改（同次运行 32 条其余用例全绿）。
- 未修改 `crates/pony-agent-core/src/bin/tui.rs`；未做任何 git 写操作。


