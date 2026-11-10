# Agent Note: TUI Ask 面板多选打通与版式重构

Status: implemented

## Problem

TUI 的 `ask_user`（模型面名 `Ask`）有「多选」的数据结构，却没有可达的多选路径：

- `AskUserQuestion.multi_select` 与 `AskUserState.multi_selected_indices` 只被面板渲染与答案取值读取；
- 真实宿主路径恒定为单选：`bin/tui/projection.rs` 的 `ask_question_from_pending` 写死
  `multi_select: false`，`ask_user` 的 JSON schema 里也没有 `multiSelect` 参数。

用户侧同时报出面板版式粗糙：无上下内边距、问题正文与「提问」chip 挤同一行、问题与选项之间无间距、
选项用省略号截断（长标签/长说明被裁掉，读不到完整选项）。

## Decision

多选从模型声明贯通到回答载荷，面板版式按「块 + 呼吸行」重排，两者都由同一组纯函数承担行口径。

多选链路（一个事实一个家）：

- `agent/tools/schema.rs`：`ask_user.input_schema` 新增可选 `multiSelect: boolean`；
- `dispatcher::ask_multi_select_from_arguments`：只认结构化布尔 `true`，字符串 `"true"` 不宽松解释；
- `PendingControlRequest.multi_select: Option<bool>`（serde `multiSelect`，`default` + `skip_serializing_if`）
  —— additive 线缆键；
- `PersistedAsk.multi_select: bool` + `pending_asks.multi_select INTEGER NOT NULL DEFAULT 0`
  （`ensure_ask_columns` 前向补列，旧库/旧记录落 `0` = 单选）：Ask 是会话状态（ADR 0074），重启后
  多选语义不得退化成单选；
- `bin/tui/projection.rs`：读 `multiSelect`，缺失按单选处理；
- 面板：Space 勾选、`▢`/`▣` 勾选框、提示「space 勾选」；单选保持 `○`/`●` 与既有键位文案；
- `ask_answer_json`：单问多值时追加 `values: [...]`（`answer` 仍为首个取值），单选载荷形状逐字不变；
  `ask_answer_values` 按选项顺序排序去重，回答序列不随勾选顺序漂移。

版式（渲染外移到 `bin/tui/ask_panel.rs`；`inline_ask_height` 与 `render_ask_inline` 共用行口径）：

- 固定行：发丝线 1 + 上内边距 1 + 标题 1 + 问答间距 1 + 键位提示 1 + 下内边距 1，中间夹问题正文与选项行；
- 问题正文独占整块，不再与「提问」chip 同行；
- 选项用 `wrap_line` 折行（续行缩进 `ASK_OPTION_INDENT = 5` 对齐标签列、右侧留 1 列
  `ASK_RIGHT_GUTTER`），禁止省略号截断；焦点行折行后整块 `SURFACE_SEL`；
- 空间不足先丢选项尾部，问题与键位提示（esc 出口）优先保留；上限仍为 40 行（≥60 列）/ 24 行（<60 列）。

同步修订的契约：`tests/tui_overlay_cancel_acceptance.rs` 的 O2 由「面板末行贴 Composer 上缘」改为
「最后一条内容行在 Composer 之上第 2 行 + 下内边距整行空白」——面板矩形仍贴色带，末行是留白。

## Alternatives considered

1. 把 `multiSelect` 塞进 `options` 载荷（数组改对象 `{multiSelect, choices}`）：落选。`options` 与桌面端
   共享形状（`AskUserToolCallCard.vue` 只认字符串数组），改形状会让桌面端把真实选项整批丢掉。
2. 只在内存控制请求上带 `multiSelect`、不落持久化列：落选。重启后未决 Ask 仍在，多选会悄悄退化成
   单选，形成「同一问题重启前后语义不同」的半契约。
3. 面板始终允许多选（不看模型声明）：落选。是/否、A/B 这类单选问题会被渲染成勾选框，用户能提交
   模型没预期的组合。
4. 选项继续省略号截断，靠拉宽终端解决：落选。用户明确要求「不能超出面板显示」，截断等于没问。

## Consequences

- 多选可用且可持久；面板可读性提升（上下留白、问题独立成行、问答分块、长选项不丢字）。
- `PendingControlRequest` / `PersistedAsk` 各增一列，`pending_asks` 需要前向补列；`ASK_COLUMN_COUNT`
  成为列索引唯一基准并配 `debug_assert`——本轮已因追加列踩过一次「`answer_json` 被读成
  `multi_select`」的静默错位，靠该守卫与 `ask_persistence_acceptance` 的 22 例守住。
- 机械验收：`tests/tui_ask_panel_layout.rs`（3 例，渲染级版式/多选标记/折行不越界）、
  `bin/tui` 的 wave43 单测（8 例，折行/高度口径/多选排序/载荷形状/投影解析）、
  `ask_persistence_acceptance`（22 例，含列迁移后读回）。
