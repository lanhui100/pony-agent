# ADR 0082：会话投影剔除宿主续轮提示——标题、轮次与 `/resume` 列表只认真实用户轮

Status: implemented

日期：2026-11-07
契约真值源：`crates/pony-agent-core/src/agent/session/tests.rs::host_continuation_prompt_recognizes_all_host_injected_turns`、
`continuation_only_turns_do_not_pollute_title_turn_count_or_resume_list`；
判定口径 `crates/pony-agent-core/src/agent/session/store.rs::is_host_continuation_prompt`

## 背景

TUI 有三类**宿主续轮**在派发时被赋予固定文案：Ask 作答后的恢复（`ASK_RESUME_CONTINUATION_PROMPT`）、
工具审批的批准/拒绝恢复（`APPROVAL_APPROVED_CONTINUATION_PROMPT` / `APPROVAL_DENIED_CONTINUATION_PROMPT`）、
goal 轮次推进（`[goal round N] 推进目标：<objective>`）。这些文案通过 `display_message` 进入
`turn_input`，被 `turn_persist` 当作普通 `user` 轮写入 `SessionStore` 的 `history`。

而 `refresh_session_metadata` 用**第一条 user 消息**做会话标题、用 **user 消息条数**做 `turn_count`，
于是续轮文案成了标题与轮次，并挤占 24 条历史上限，把原始用户轮挤出库。真实用户库中已可见污染：
标题变成「（已作答，继续上一轮未完成的步骤）」「（已批准该工具调用，请重试刚才的操作并继续任务）」，
goal 会话标题变成「[goal round 4] 推进目标：…」；更有一类会话**只有续轮提示 + assistant**，
从未有过真实用户轮，却照样在 `/resume` 里当作「历史会话」出现。

## 候选方案与拒绝理由

- **A. 落库时就不写这些轮**：拒绝（本次不做）——`display_message` 同时是运行时续轮输入的一部分，
  在持久化层按文案丢弃会改变 turn 序列与 `prepare_turn` 的输入形状，风险高于收益；
  且历史库里已有污染数据不会因此消失。
- **B. 迁移脚本一次性清洗 SQLite**：拒绝——一次性脚本修不了「下一次续轮继续污染」，
  且需要用户手动执行、跨库版本维护。
- **C. 只在 TUI 渲染层隐藏**：拒绝——`turn_count`／标题是**库内派生列**，隐藏渲染会让
  其他读投影的入口（侧边栏、`/resume`、指标）继续拿到被污染的值，属于把缺陷推到别处。
- **D. 采用方案**：在**会话投影层**（`session/store.rs`）建立唯一判定口径
  `is_host_continuation_prompt`，让标题/轮次/trace 兜底标题一律跳过续轮；新增投影字段
  `continuation_only` 供「以用户会话为语义」的入口（`/resume`）过滤碎片；启动加载时把纠正后的
  标题/轮次回写，污染数据自愈。

## 决策

1. **唯一判定口径**：`is_host_continuation_prompt(text) -> bool` 放在 `session/store.rs`，
   逐字匹配三条固定文案（允许首尾空白）+ 结构匹配 `[goal round <数字>] 推进目标：`。
   三条固定文案常量同时作为**唯一真值源**从 `session` 模块导出，`bin/tui.rs` 改为引用库常量，
   杜绝两处文案漂移。
2. **标题**：`build_title` 取**第一条非续轮**的 user 消息；纯 trace 会话（history 未落库）
   同样从尾部反向找**非续轮** trace 标题，全为续轮时回落到 `DEFAULT_SESSION_TITLE`。
3. **轮次**：`turn_count` 只统计非续轮 user 消息；`title_override` 语义不变。
4. **碎片投影**：`SessionOverview` 新增 `continuation_only: bool`（`#[serde(default)]`，向后兼容），
   判据 `session_is_continuation_fragment`——见过续轮、且**没有任何**真实用户轮（history 无真实
   user 消息时回落到 trace 标题判定）。
5. **自愈回填**：`SessionStore` 加载循环里比较 `refresh_session_metadata` 前后的
   `title` / `turn_count`，有变化即置 `should_save`，把纠正值写回库；持久化 schema 不变，
   只更正既有列取值。
6. **`/resume` 列表**：过滤 `!continuation_only`，行右侧改为「N 轮 · 时间 · 短 id」，
   时间口径为同日 `HH:MM` / 同年 `MM-DD HH:MM` / 跨年 `YYYY-MM-DD`，缺失或越界回退 `—`。

## 影响

- **代码面**：`crates/pony-agent-core/src/agent/session/store.rs`（判定、标题、轮次、回填、投影）、
  `types.rs`（`SessionOverview.continuation_only`）、`session/mod.rs`（导出）、
  `crates/pony-agent-core/src/bin/tui.rs`（常量引用库真值源、`/resume` 过滤与行布局）。
- **契约面**：`SessionOverview` 为**加字段**的向后兼容演进（`serde(default)`）；
  会话标题/轮次的语义收紧为「真实用户轮」，`continuation_only=true` 的会话不再出现在 `/resume`。
- **不改的部分**：Ask/审批/goal 的续轮派发链路与 `display_message` 仍原样写库（方案 A 留待将来），
  因此**原先已被挤出的历史用户轮不会复原**——本次只保证新会话不再污染、旧库标题/轮次自愈。
- **§3 观测性**：结构性 N-A（本地 SQLite 投影 + 终端渲染，无新增对外接口 / 网络 IO / 后台任务 /
  LLM 调用），登记见 `docs/observability.yaml` 的
  `wave-tui-session-continuation-projection` 条目。

## 验收

| 契约 | 断言 |
| :-- | :-- |
| C1 三类续轮文案 + goal 轮次结构全部命中，真实输入不误判 | `host_continuation_prompt_recognizes_all_host_injected_turns` |
| C2 只有续轮提示的会话：标题回落默认值、轮次为 0、`continuation_only=true` | `continuation_only_turns_do_not_pollute_title_turn_count_or_resume_list` 前半 |
| C3 追加真实用户轮后：标题为该轮、轮次为 1、碎片标记消失 | 同上后半 |
| C4 加载即自愈：纠正后的标题/轮次回写库，重启不再重算 | `SessionStore` 加载循环 `should_save` 分支 |
