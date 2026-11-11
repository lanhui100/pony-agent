# Agent Note: Ask 内联面板高度随问题换行增长，硬上限按终端宽度分档（40 / 24）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0081-tui-inline-ask-height-adaptive.md` 之归档快照，仅供溯源。

日期：2026-11-07
契约真值源：`crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs::o3_ask_panel_survives_wide_narrow_and_tiny_terminals`
与 `crates/pony-agent-core/src/bin/tui.rs` 的 `wrap_ask_question` / `inline_ask_height` 单测
（`test_inline_ask_height_grows_with_wrapped_question`、`test_wrap_ask_question_*`）

## 背景

旧实现把 Ask 面板高度写成 `(2 + option_rows + 1).min(cap).max(4)`，`cap = if width >= 60 { 12 } else { 8 }`，
即：**高度只由选项条数决定，与问题正文的实际换行行数无关**。这带来两个可复现缺陷：

1. **长问题被裁掉**：真实提问常常是一整段（含约束条件、验收标准），宽终端下换行后可达十几行，
   面板却恒为 12 行封顶 ⇒ 问题尾部被裁，用户只能凭猜测回答。
2. **窄终端更糟**：`cap = 8` 比宽终端更小，而窄终端恰恰是换行更多、最需要高度的地方——上限方向反了。

面板是 ADR 0063 的覆盖层（不参与消息区几何），所以放大它的唯一风险是**吞掉整屏**，
而这一风险已经由 `ui()` 的二次收缩（不得越过 header / composer）承担。

## 候选方案与拒绝理由

- **A. 去掉上限，完全自适应**：拒绝——一行超长文本（无换行符）在极窄终端可算出上百行，
  面板会覆盖整屏，语义上等价于「把历史全遮掉」，且 `u16` 溢出风险需要额外防护。
- **B. 面板内滚动问题正文**：拒绝——增加一层滚动状态机与键位（`PgUp/PgDn` 与选项焦点冲突），
  而问题正文通常只需十几行，收益远小于复杂度。
- **C. 对问题正文做省略号截断**：拒绝——问题必须完整可读；截断会让「验收标准」这类关键尾巴消失，
  用一个缺陷换另一个缺陷。
- **D. 沿用 `cap 12/8` 只把问题行数计入**：拒绝——计入后必然撞上旧 cap，等于没修；上限本身必须抬高。
- **E. 采用方案**：问题正文按面板真实可用宽度换行并计入高度；硬上限按「宽 ≥60 ⇒ 40 行 / 窄 ⇒ 24 行」
  分档，保留 `max(4)` 下限，最终仍由 `ui()` 收缩。

## 决策

1. **问题换行用专用纯函数** `wrap_ask_question(question, first_width, rest_width) -> Vec<String>`：
   CJK 宽度感知（`char_width`），首行与续行宽度可不同，沿用 `wrap_line` 的行首避让规则
   （`is_no_line_start` 标点不落行首），**绝不使用省略号截断**；空问题返回单条空串，
   保证调用方至少有一行可渲染。
2. **宽度口径与渲染一致**：`prefix = RAIL + 2 + 1 + 4 + 1 + header_width`（轨道 + 呼吸 + 提问 chip +
   分组 chip），`limit = width.saturating_sub(prefix).max(8)`；`header_width` 与 `render_ask_inline`
   同口径过滤（去空白为空、含 `:`、以 `builtin` 开头 ⇒ 不占宽）。
3. **高度公式**：`needed = 2 + option_rows + 1 + (question_rows - 1)`——
   `2` = 分隔线 + 标题，`option_rows = 选项数 + 1`（自定义输入项），`1` = 键位提示行，
   `question_rows - 1` = 问题换行带来的**额外**行（首行已隐含在标题行）。
4. **硬上限分档**：`cap = if width >= 60 { 40 } else { 24 }`；结果 `(needed).min(cap).max(4)`。
   方向性理由：宽终端能容纳更多行，且真实长问题多出现在宽终端；窄终端上限仍显著高于旧的 8 行。
5. **上限只是防吞屏保险**：`ui()` 仍会按 `chat_rect` / `bottom_top` 二次收缩（不得越过 header、
   不得挤掉 composer），因此 cap 不需精确等于可视高度。
6. **极小终端允许降级**：高度不足以容纳全部内容时，**优先保留问题正文与 `esc` 提示**
   （选项可被裁），不允许 panic 或越出终端边界。

## 影响

- **代码面**：`crates/pony-agent-core/src/bin/tui.rs`——新增 `wrap_ask_question`、重写 `inline_ask_height`、
  `render_ask_inline` 复用同一换行结果（宽度口径单源）。
- **纯本地渲染**：无网络 IO、无 LLM 调用、无后台任务、无对外接口、无持久化 schema 变更。
- **既有契约**：ADR 0063（Ask 覆盖层与取消反馈）的交互语义、ADR 0042 的 composer 几何、
  ADR 0071 的流式期面板可开均未改写；仅「面板高度上界」与「问题正文可完整展示」两条内容契约变化。
- **失效的旧断言**：`Ask 面板高度必须落在 4..=12`（宽）/ `4..=8`（窄）作废，改为 `4..=40` / `4..=24`
  并新增「长问题换行后必须突破 12 行」的方向性断言。
- **§3 观测性**：结构性 N-A（纯渲染层几何，无新增对外接口 / 网络 IO / 后台任务 / LLM 调用）。

## 验收

| 契约 | 断言 |
| :-- | :-- |
| C1 宽终端（120 列）高度落在 `4..=40`，且长问题换行后 `> 12` | `o3` 前两段 |
| C2 宽终端长问题在屏上仍可见（`LONGQ_SENTINEL`） | `o3` |
| C3 窄终端（56 列）高度落在 `4..=24` | `o3` |
| C4 窄终端空间不足时优先保留问题与 `esc` 提示 | `o3` |
| C5 极小终端（40x10）不 panic、不越界 | `o3` 末段 |
| C6 换行函数：短路单行、标点不落行首、CJK 宽度不溢出 | `tui::tests::test_wrap_ask_question_*` |
| C7 高度随换行增长 | `tui::tests::test_inline_ask_height_grows_with_wrapped_question` |

回归：`tui_overlay_cancel_acceptance`（o1–o8）、`tui_six_fixes_acceptance`、`tui_render_layout`。

## 关联

- 承接 ADR 0063（Ask 覆盖层）与 ADR 0071（流式期面板可开）；
- 与 ADR 0078（header / 底面行分层）、ADR 0079（过程折叠）同波次落地。

