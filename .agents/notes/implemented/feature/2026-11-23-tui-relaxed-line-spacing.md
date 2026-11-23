# Agent Note: TUI 消息可选宽松留白（relaxed_spacing）

Status: implemented

## Problem

终端行高由用户字体决定，ratatui 只有细胞网格、没有行距属性；`message_lines_with_ctx` 的助手正文
逐行 `push_prefixed`，相邻行完全贴死。长段落在高分屏或大行高字体下读起来很密，用户希望能
「增加少许行距」。既有的轮间距决策（`2026-11-07-tui-user-message-vertical-gap.md`，ADR 0088）
只解决消息边界与气泡内衬，不解决段内留白。

另有一条硬约束必须先摆明：**细胞网格里「行距」只能是整行倍数**（0 行或 1 行 ⇒ 0% / 100%），
25% / 50% 在应用侧根本不存在。故「少许」只能通过**收窄插行面**逼近，而不能靠调小比例达成。

## Decision

1. `TuiPreferences` 新增 `relaxedSpacing: bool`（默认 `false`；旧偏好文件缺字段按紧凑走，向前兼容）。
2. `TuiState.relaxed_spacing` 参与 `RenderCache` 失效判据（与 `hide_process` / `thinking_collapsed` 同口径：
   开关翻转必须整体重排，只重算末条会让历史停在旧行距上），并瞬时贴底避免逐帧追赶。
3. 开启后 `message_lines_with_ctx` **只在块边界加厚留白**：源空行 `push_blank()` → `push_gap(2)`；
   ATX 标题（`#`~`######` + 空格，判据 `is_markdown_heading_line`）前后各 1 行呼吸。**正文相邻行一律不插行**
   —— 连续正文下宽松与紧凑的渲染行数完全相同（视口密度不退化，见测试
   `relaxed_does_not_change_dense_body_line_count`）。
4. 标题呼吸用 `push_blank()`（自带「上一行非空才插」去重）：段首 / 已有空行处不会叠行；
   标题后紧跟源空行时由段间分支统一收敛为 2 行，不叠成 3 行。
5. 首版（同日修正）曾按「正文逐行插 1 行 + 代码块/表格/列表豁免」落地，实测即 100% 行距、
   同屏正文行数腰斩。收窄后豁免概念消失，判定函数 `is_markdown_list_item_line` 一并删除（无消费者）。
6. 唯一入口是 `/settings` 面板新增的「宽松行距」项（`Space` / `Enter` 切换），与
   `hide_process` / `thinking_collapsed` 共用同一落盘点（`persist_tui_preferences`）与提示文案风格；
   未新增快捷键，避免与既有键位及终端自身语义争用。
7. 纯函数入口 `message_lines_for` 固定使用紧凑基线（`relaxed_spacing: false`），
   契约 P-12「同一消息首次出现与单独渲染行内容一致」不因用户偏好漂移。

## Alternatives considered

- **正文逐行插 1 行（首版口径，同日被收窄）**：细胞网格里那等于 100% 行距，同屏可见正文行数腰斩、
  滚动总量翻倍，用户实测「松得过头」。收窄插行面（只花在段间与标题上）比降低比例更接近「少许」。
- **半行 / 像素级行距**：ratatui 细胞网格没有行高属性，做不到。伪造「半行」只能自画 `▀▄` 半块字符，
  且会把复制出来的正文污染成噪声（同一理由曾否决用户消息竖线，见 ADR 0088）。
- **kitty graphics / sixel 自绘文本**：能拿到真实像素级行高，但会同时失去选中、复制、搜索与无障碍，
  代价远超收益；OSC 66 文本缩放协议只有少数终端支持，crossterm 亦不解析。
- **默认开启宽松**：所有既有布局断言与观感一次性改变；且 `message_lines_for` 的测试基线会与新默认分叉，
  破坏 P-12 一致性。故做成显式开关，默认紧凑。
- **在 `ui()` 逐帧循环里缓存外注入空行**：渲染缓存按消息切块，缓存外插行会让缓存行数与真实高度分裂。
- **配一个快捷键（如 Ctrl+L）**：与终端自身语义及既有键位争用，收益不足；设置面板已能一键切换并持久化。

## Consequences

- 代码：`crates/pony-agent-core/src/agent/tui_preferences.rs`（字段 + 默认 + 序列化兼容）、
  `crates/pony-agent-core/src/bin/tui.rs`（状态字段、缓存失效判据、`/settings` 行与切换分支、
  `MessageRenderCtx.relaxed_spacing`、正文渲染的块边界加厚分支、`is_markdown_heading_line`）。
- 测试：`crates/pony-agent-core/src/bin/tui/tests/relaxed_spacing_tests.rs`（9 例：紧凑基线不变 /
  宽松下正文行仍紧邻 / 连续正文两口径行数相同 / 段间 2 行 / 标题前后各 1 行 / 标题+源空行不叠 3 行 /
  代码块紧邻 / 连续列表项紧邻 / `settings` 切换往返）；`tests/tui_process_fold_acceptance.rs` 三处
  `TuiPreferences` 字面量补字段、面板说明文案断言同步；`agent::tui_preferences::tests` 补默认值与往返断言。
- 默认关闭 ⇒ 既有渲染行数、滚动锚定、选区映射与全部既有断言口径不变（紧凑路径仍是同一份实现）。
- 残余风险：宽松只作用于段间与标题，因此对「一整段没有任何空行、也没有标题」的长段落**视觉上几乎无感**
  （这是收窄口径的必然代价，也是「少许」的定义）；若后续需要更强的段内呼吸，只能回到逐行插行，
  代价是同屏行数腰斩。
- 若后续引入消息分块缓存或行级虚拟化，块边界加厚必须继续留在 `message_lines_with_ctx` 的块内，
  否则缓存行数与几何会再次分裂。
