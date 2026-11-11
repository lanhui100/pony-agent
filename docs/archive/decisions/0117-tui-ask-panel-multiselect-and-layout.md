# ADR 0117：TUI Ask 面板多选与版式重构（内边距 / 问题独占一行 / 选项折行）

Status: implemented

## 背景

TUI 的 `ask_user`（模型面名 `Ask`）面板此前**只在数据层**预留了多选字段（
`AskUserQuestion.multi_select` + `AskUserState.multi_selected_indices`），但真实宿主路径把多选
恒定为 `false`：`projection.rs` 的 `ask_question_from_pending` 写死 `multi_select: false`，
`ask_user` 的 JSON schema 也没有 `multiSelect` 参数。于是「多选」在用户可见面上不存在——只能单选。

同一次用户实测还指出面板版式粗糙：

1. 面板无上下内边距，首末行贴着发丝线与 Composer 色带；
2. 问题正文与「提问」chip 挤在同一行，长问题换行后每行都是「chip + 半句问题」的碎读；
3. 问题与选项之间没有呼吸行；
4. 选项用省略号**截断**（`ellipsize_width`），长标签/长说明被裁掉，用户看不到完整选项。

## 决定

### 1. 多选端到端打通（一个事实一个家）

| 层 | 落点 | 语义 |
|---|---|---|
| 模型面 schema | `agent/tools/schema.rs` 的 `ask_user.input_schema` 新增 `multiSelect: boolean`（可选） | 模型声明「这个问题可以同时选多项」 |
| 请求提取 | `dispatcher::ask_multi_select_from_arguments` | 只认结构化布尔 `true`；缺失 / `false` / 字符串 `"true"` ⇒ 单选 |
| 内存控制请求 | `PendingControlRequest.multi_select: Option<bool>`（serde `multiSelect`，`default` + `skip_serializing_if`） | additive 线缆键，旧消费方不读即不变 |
| 持久化 | `PersistedAsk.multi_select: bool` + `pending_asks.multi_select INTEGER NOT NULL DEFAULT 0`（`ensure_ask_columns` 前向补列） | 重启后多选语义不丢；旧库 / 旧记录落 `0` = 单选 |
| TUI 投影 | `ask_question_from_pending` 读 `multiSelect` | 缺失 ⇒ 单选（与历史行为逐字一致） |
| TUI 交互 | Space 勾选（`toggle_current_option`）、`▢`/`▣` 勾选框、提示「space 勾选」 | 单选仍用 `○`/`●`，键位提示逐字不变 |
| 回答载荷 | `ask_answer_json`：单问多值时追加 `values: [...]`，`answer` 仍为首个取值 | 模型能读到**全部**勾选项；单选载荷形状逐字不变 |

多选答案按**选项在面板中的顺序**输出（`ask_answer_values` 排序去重），不按勾选顺序——否则
「先勾 B 再勾 A」会让模型收到的答案序列随机化。

### 2. 面板版式（行口径唯一）

面板行结构固定为：`发丝线 1 + 上内边距 1 + 标题 1 + 问题正文 ≥1 + 问答间距 1 + 选项行 + 键位提示 1 + 下内边距 1`。
面板渲染整体外移到独立子模块 `bin/tui/ask_panel.rs`（`bin/tui.rs` 只保留 `#[path] mod ask_panel` +
`pub use`，行数净减），`inline_ask_height` 与 `render_ask_inline` **共用**同一组纯函数（`ask_option_rows` /
`ask_custom_input_label` / `ask_key_hint`），高度不会与实际渲染行数分叉。

- 选项不再省略：`ask_option_rows` 用 `wrap_line` 折行，续行缩进 `ASK_OPTION_INDENT = 5`
  与标签列对齐，右侧留 1 列 `ASK_RIGHT_GUTTER`；焦点行折行后整块 `SURFACE_SEL`。
- 空间不足时**先丢选项尾部**，问题正文与键位提示（含 esc 出口）优先保留。
- 上限仍是 40 行（≥60 列）/ 24 行（<60 列），`ui()` 继续按消息区几何二次收缩。

## 候选方案

1. **把 `multiSelect` 塞进 `options` 载荷（数组改对象 `{multiSelect, choices}`）** —— 落选：
   `options` 的线缆形状是与桌面端共享的契约（`src/components/ask/AskUserToolCallCard.vue` 只认
   字符串数组），改形状会让桌面端把真实选项整批丢掉（静默降级为只有自由输入）。
2. **不落持久化列，只在内存控制请求上带 `multiSelect`** —— 落选：Ask 是**会话状态**（ADR 0074），
   重启后未决请求仍在；不持久化会让重启后的多选问题悄悄退化成单选，形成「同一问题重启前后语义
   不同」的半契约。
3. **面板始终允许多选（不区分模型的 `multiSelect`）** —— 落选：单选问题（是/否、A/B）会被渲染成
   勾选框，用户可提交模型没预期的组合；面板语义必须由模型声明决定。
4. **选项继续省略号截断，靠横向拉伸终端解决** —— 落选：用户明确要求「不能超出面板显示」，且
   截断后用户读不到完整选项等同于没问。

## 后果

- 正面：多选能力从模型声明一路贯通到回答载荷；面板可读性与留白符合终端阅读习惯；长选项不再丢字。
- 代价：`PendingControlRequest` / `PersistedAsk` 各增一列，`pending_asks` 表需要前向补列（已由
  `ensure_ask_columns` 覆盖，旧库补 `0`）；`ASK_COLUMN_COUNT` 常量成为列索引的唯一基准，配
  `debug_assert` 防漂移（本轮已因追加列踩过一次「`answer_json` 被读成 `multi_select`」）。
- 契约修订：`tests/tui_overlay_cancel_acceptance.rs` 的 O2 由「面板末行贴 Composer 上缘」改为
  「最后一条内容行在 Composer 之上第 2 行 + 下内边距整行空白」——**面板矩形仍贴色带**，只是末行
  是留白，这是本次版式要求的直接后果。
- 机械验收：`tests/tui_ask_panel_layout.rs`（3 例，渲染级）+ `bin/tui` 的 wave43 单测（8 例，
  折行/高度/多选标记/载荷形状）+ `ask_persistence_acceptance`（22 例，含列迁移后的读回）。
