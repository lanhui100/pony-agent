# ADR 0088：TUI 用户消息上下各留 1 行页面底色间距

Status: implemented

## 背景

`pony` TUI 里用户自己发的消息与相邻内容视觉贴边：

- 用户气泡（`push_user_block`）由 1 行 SURFACE 上内衬 + 正文 + 1 行 SURFACE 下内衬组成，
  内衬是「气泡材质」而不是「轮间呼吸」。SURFACE(`#1B1916`) 与 CANVAS(`#12100E`) 对比很弱，
  贴边时读者分不清「消息内容结束」与「下一条消息开始」。
- 既有轮间距全部由**非用户**角色承担：`Assistant` 分支的 `push_gap(2)` / `push_blank`、
  `Thinking` 块尾 1 行呼吸（ADR 0076）。`MessageRole::User` 分支本身**不产生任何空白行**，
  于是 User↔Assistant、User↔System（状态消息）、User↔ToolCall/ToolResult 的间距全靠
  对侧块提供；对侧没有间距时（例如工具行紧接用户消息、状态消息紧接用户消息）就贴边。
- 用户诉求：用户消息上方与下方，与其它消息（含状态消息与 agent 消息）之间各多 1 行间距。

## 候选方案 / Alternatives considered

- **A. 把气泡内衬从 1 行加到 2 行**（改 `push_user_block`）：落选。内衬是气泡材质的一部分，
  加深会被读成「更大的气泡」而不是「轮与轮之间的呼吸」；且用户块是唯一带 SURFACE 的角色，
  放大材质会进一步拉开与 agent 正文的视觉权重差。
- **B. 复用既有 `push_gap(2)` 轮间距**：落选。`push_gap` 只在块内已有前序行时生效，
  而 `message_lines_with_ctx` 每条消息返回**独立 `Vec`**，块首调用恒为 no-op；
  且它按「补齐到 n 行空白」计数，与 User 相邻的既有 1 行呼吸会被吞成同一行或叠成 2 行，
  破坏 ADR 0076 / 契约 C6 已确定的节奏。
- **C. 在 `message_lines_with_ctx` 的 `MessageRole::User` 分支前后各插 1 行 `blank()`**：采用。
  间距归属用户块自身 ⇒ 不依赖对侧角色，与对侧既有间距自然相加，语义单一。
- **D. 在 `ui()` 的逐帧循环里按 role 注入间距**：落选。渲染缓存（`render_cache`）按消息切块缓存，
  在缓存之外注入会让「缓存行数」与「真实渲染高度」分裂，滚动锚定与窗口切片随即失真。
- **E. 改成「用户消息包一层上下 padding 的容器块」**：落选（本轮）。需要新的块模型与缓存键，
  收益只是形式统一，不改任何可见几何，超出最小闭包。

## 决策

1. **上方间距有条件**：`ctx.has_preceding_lines == true` 时，User 分支先 push 1 行 `blank()`
   （页面底色）。会话首条用户消息上方不得凭空多出一行。
2. **下方间距无条件**：User 块尾部恒 push 1 行 `blank()`。若它恰好是会话最后一行，
   `ui()` 的 `scroll_total` 尾部连续空白剔除（ADR 0076）会让它不参与滚动锚定，不产生悬空留白。
3. **删除 User 分支块首的 `push_gap(2)`**：在独立 `Vec` 上恒为 no-op 的死代码，留给读者的是
   错误的心智模型（以为轮间距由它负责）。
4. **气泡几何不变**：`push_user_block` 的 1 行上内衬 / 1 行下内衬保持原样，材质与正文排版零改动。
5. **可观测几何**（验收判据）：
   - 相邻消息行差由 2 变 3：中间两行 = 1 行 CANVAS（新增间距）+ 1 行 SURFACE（气泡内衬）；
   - 会话首条用户消息块总行数 = 4（上内衬 + 正文 + 下内衬 + 下间距）；
   - 上方间距恒落在 CANVAS，内衬恒为 SURFACE —— 两者不可互相顶替。

## 影响与残余风险

- 改动点：`crates/pony-agent-core/src/bin/tui.rs` 的 `MessageRole::User` 分支（唯一产品代码改动）。
- 既有断言改判：`crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs` 的 A12 ——
  User→Thinking 行差 2→3、Thinking→User 行差 3→4（思考块自身呼吸不变）。
- 新增验收：`crates/pony-agent-core/tests/tui_user_block_gap.rs`，
  覆盖 agent 正文 / 状态消息 / 工具行三类邻接，以及「首条用户消息无顶部空行」。
- 残余风险：`MessageRole::User` 的渲染分支目前仍在单文件 `tui.rs` 内（未随 `tui/` 目录重构外移）。
  若后续把消息渲染域拆到独立模块，本决策的注释与 ADR 引用必须随迁，否则间距语义会变成孤儿实现。
- `hide_process`（契约 C9）不受影响：被隐藏的过程消息返回空 `Vec`，不产生行，
  `has_preceding_lines` 仍按「前面是否已有非空块」判定。
