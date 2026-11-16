# Agent Note: TUI 右侧滚动条补上鼠标命中与拖拽

Status: implemented

## Problem

用户报告：TUI 主会话消息区右侧的滚动条用鼠标拖不动，只能靠滚轮/按键滚动。

根因不是配置或终端能力，而是**实现只做了一半**：`crates/pony-agent-core/src/bin/tui.rs` 里滚动条有完整渲染（契约 C1 `scrollbar_thumb` + C2/C3 `render_scrollbar` 逐格写 `│`/`█`），但鼠标事件链路里完全没有滚动条：

1. `render_scrollbar` 把轨道固定在**终端最右列** `x = screen.width - 1`，而正文矩形 `chat_rect` 恒为这一列让位（`content_w = min(width - 6, 120)`，右缘 ≤ `width - 2`）。
2. 鼠标 `Down(Left)` 只调 `selection_point_at`，它要求命中 `chat_rect` 或输入框矩形。滚动条列永远在两者之外 ⇒ 按下即判为「非选中区」，只做清选区，不产生任何滚动动作。
3. `Drag(Left)` 只在 `state.mouse_selection.active` 为真时才处理，`Up(Left)` 同样只收选区会话。

于是「按下 → 拖动 → 抬起」整条链在最右列被静默丢弃：滚动条成了纯装饰，视觉上可点可拖，实际无任何响应。这类缺陷不会被布局/几何测试发现——既有验收（`tests/tui_process_fold_acceptance.rs` A1–A3）只断言「画在哪、画不画」，没有断言「点得动」。

## Decision

在 `bin/tui.rs` 内补齐滚动条的鼠标语义，全部与既有绘制几何同源，不引入第二份几何推导。

> 关联既有决策：滚动条几何（C1–C3）与「阻尼跟随 / 贴底吸附」的语义见
> `.agents/notes/implemented/bug-fix/2026-11-08-tui-ctrl-o-scroll-snap.md`；本条只在其上补鼠标入口，
> 不改跟随与吸附判据。

1. **反解纯函数 `scrollbar_offset_for_row(total, visible, row, grab) -> Option<u32>`**：`scrollbar_thumb`（`offset → 滑块起点`）的逆运算，取整口径与绘制严格一致（`start = round(offset * track / denom)` ⇔ `offset = round(start * denom / track)`）。偏移按 `start = clamp(row - grab, 0, track)` 饱和，**`row` 本身不截断**：指针拖到轨道之外时仍需继续朝末端饱和，否则「抓住滑块中部往下拖」会永远差 `grab` 行贴不到底。`None` 表示没有轨道（内容不溢出）→ 调用方不得改偏移。
2. **命中函数 `scrollbar_hit(state, x, y) -> Option<(row, grab)>`**：只认渲染回写的 `last_scrollbar_rect`（`render_scrollbar` 在真正画格子的同一处写 `Some(Rect{ x, y, width: 1, height: chat_rect.height })`），因此「画了什么」与「能点到什么」永远同源；`ui()` 每帧先置 `None`，面板态/不溢出态不会残留上一帧的列矩形。`grab` = 指针相对滑块顶部的行偏移（指针在滑块内），或 `thumb_len / 2`（点在轨道空白 ⇒ 滑块中心吸附到指针，即「点哪跳哪」）。
3. **唯一写入口 `apply_scrollbar_row(state, row, grab)`**：折算偏移并写 `scroll_offset`；落到底部恢复 `auto_scroll` 贴底跟随，非底部解除跟随（与滚轮/方向键同一判据）。
4. **事件链路优先级**：`Down(Left)` 先试滚动条命中——命中则置 `scrollbar_drag = Some(grab)`、立即吸附并把滑块拖到指针行、清空文本选区（两种手势互斥）；`Drag(Left)` 若 `scrollbar_drag` 活跃则按 `y - rect.y` 继续折算（越出轨道由反解饱和，指针跑到窗口外仍能滚到首/末）；`Up(Left)` 结束会话。轨道消失（面板打开、内容缩到不溢出）或进入 Ask 覆盖层时强制收口会话，避免一直吞掉后续正文拖选。
5. **保留抓取偏移**：`grab` 存下来是为了「跟手」——拖住滑块中部时滑块与指针的相对位置不变，否则每次 Drag 都会把滑块顶对齐到指针，出现瞬跳。
6. **验收**：新增 `crates/pony-agent-core/tests/tui_scrollbar_drag.rs`（8 例，A1 反解口径/两端精确 + 一行颗粒度、A2 命中只认已绘制几何且不侵占正文列、A3 拖到首末与中点的写入语义与幂等）。

## Alternatives considered

- **A. 放大滚动条命中区（最右列 + 相邻 1–2 列都算轨道）**：落选。会让「右边缘正文列」的文本选区变成滚动，用户拖选句子末尾时莫名跳页；且与契约 C2「不侵占正文列」冲突。宁可命中窄但确定。
- **B. 复用 `chat_rect` 推导滚动条列（`x = chat_rect.x + chat_rect.width`）**：落选。超宽终端下内容居中，正文右缘与终端最右列之间恒有 `GUTTER = 3` 列留白，推导出的列与实际绘制不符（差 2–3 列），命中会整体偏移。
- **C. 只做「点击轨道跳转」不做拖拽**：落选。用户诉求正是「拖不动」；且轨道只有点击能力时，长会话逐页点击比滚轮更差，等于没修。
- **D. 在渲染期缓存 `thumb_start/thumb_len` 供命中复用**：落选。两者都由 `(total, visible, offset)` 纯函数决定，缓存只增加「缓存与实际不同步」的风险面；现场用同一 `scrollbar_thumb` 重算即同源。
- **E. 抽到独立模块 `tui/scrollbar.rs`**：本轮落选。`scrollbar_thumb` 已被 `tests/tui_process_fold_acceptance.rs` 以 `tui::scrollbar_thumb` 路径引用（冻结契约），移动会改变公开路径；`tui.rs` 现有文件级 ponylint 豁免仍在有效期内，收益不足以支付路径迁移成本。

## Consequences

- 滚动条从「纯呈现」变成真实交互控件；正文选区、滚轮、方向键行为不变（滚动条列原本就不属于任何可选中区域）。
- 命中依赖「上一帧真的画了滚动条」，因此面板态、空态、不溢出态自动不可拖——无需在每个模式切换点手工同步可见性。
- 反解是多对一映射（`track` 行表达 `denom + 1` 个偏移），中间位置存在不超过一行的颗粒度误差；两端（顶/底）精确对齐，这是刻意选择，避免「拖到底却差一行」。
