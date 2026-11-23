# Agent Note: TUI 正文可选宽松行距（relaxed_spacing）

Status: implemented

## Problem

终端行高由用户字体决定，ratatui 只有细胞网格、没有行距属性；`message_lines_with_ctx` 的助手正文
逐行 `push_prefixed`，相邻行完全贴死。长段落在高分屏或大行高字体下读起来很密，用户希望能
「增加少许行距」。可用的唯一手段是在渲染行之间**插空行**，但无条件插空行会把代码块、表格与
连续列表项拆散，视口效率也直接腰斩。既有的轮间距决策（`2026-11-07-tui-user-message-vertical-gap.md`，
ADR 0088）只解决消息边界与气泡内衬，不解决段内行距。

## Decision

1. `TuiPreferences` 新增 `relaxedSpacing: bool`（默认 `false`；旧偏好文件缺字段按紧凑走，向前兼容）。
2. `TuiState.relaxed_spacing` 参与 `RenderCache` 失效判据（与 `hide_process` / `thinking_collapsed` 同口径：
   开关翻转必须整体重排，只重算末条会让历史停在旧行距上），并瞬时贴底避免逐帧追赶。
3. 开启后 `message_lines_with_ctx` 的助手正文行之间插 **1 行**呼吸；源空行处改用 `push_gap(2)`，
   保证段间距（2 行）严格大于行距（1 行），段落不会被读成一串连续行。
4. 豁免面保持紧凑：代码块（含陶土底色内衬行）、GFM 表格块、连续列表项（`- ` / `* ` / `+ ` /
   `1. ` / `1) ` 及缩进续行）。判定函数 `is_markdown_list_item_line` 是纯函数，缩进续行由
   「上一源行也是列表项」并入同一豁免。
5. 唯一入口是 `/settings` 面板新增的「宽松行距」项（`Space` / `Enter` 切换），与
   `hide_process` / `thinking_collapsed` 共用同一落盘点（`persist_tui_preferences`）与提示文案风格；
   未新增快捷键，避免与既有键位及终端自身语义争用。
6. 纯函数入口 `message_lines_for` 固定使用紧凑基线（`relaxed_spacing: false`），
   契约 P-12「同一消息首次出现与单独渲染行内容一致」不因用户偏好漂移。

## Alternatives considered

- **默认开启宽松行距**：所有既有布局断言与观感一次性改变、视口利用率腰斩；且 `message_lines_for`
  的测试基线会与新默认分叉，破坏 P-12 一致性。故做成显式开关，默认紧凑。
- **每行都插空行（含代码块 / 表格 / 列表）**：代码块被拆成隔行、表格块断裂、清单一条占两行，
  可读性收益被结构损失抵消。
- **半行 / 像素级行距**：ratatui 细胞网格没有行高属性，做不到；伪造「半行」只能自画字符，
  会把复制出来的文本污染成噪声（同一理由曾否决用户消息竖线，见 ADR 0088）。
- **在 `ui()` 逐帧循环里缓存外注入空行**：渲染缓存按消息切块，缓存外插行会让缓存行数与真实高度分裂。
- **配一个快捷键（如 Ctrl+L）**：与终端自身语义及既有键位争用，收益不足；设置面板已能一键切换并持久化。

## Consequences

- 代码：`crates/pony-agent-core/src/agent/tui_preferences.rs`（字段 + 默认 + 序列化兼容）、
  `crates/pony-agent-core/src/bin/tui.rs`（状态字段、缓存失效判据、`/settings` 行与切换分支、
  `MessageRenderCtx.relaxed_spacing`、正文渲染分支、`is_markdown_list_item_line`）。
- 测试：新增 `crates/pony-agent-core/src/bin/tui/tests/relaxed_spacing_tests.rs`（紧凑基线不变 /
  正文行距恰 1 行 / 段间距 2 > 1 / 代码块与连续列表豁免 / `settings` 切换往返）；
  `tests/tui_process_fold_acceptance.rs` 三处 `TuiPreferences` 字面量补字段；
  `agent::tui_preferences::tests` 补新字段默认值与往返断言。
- 默认关闭 ⇒ 既有渲染行数、滚动锚定、选区映射与全部既有断言口径不变（紧凑路径仍是同一份实现）。
- 残余风险：宽松行距下同屏可见正文行数约减半、滚动总量约翻倍（用户显式选择）；
  若后续引入消息分块缓存或行级虚拟化，插空行必须继续留在 `message_lines_with_ctx` 的块边界内，
  否则缓存行数与几何会再次分裂。
