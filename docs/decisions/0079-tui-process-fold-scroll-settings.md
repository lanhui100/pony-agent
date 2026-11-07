# ADR 0079：TUI 右侧滚动条、思考折叠、`/settings` 面板与 Agent 过程默认隐藏

Status: implemented

日期：2026-11-07
契约真值源：`.dev-team/contract-matrix-tui-process-fold-scroll-settings.md`（C1–C12，含修订 R1）

## 背景

用户在同一波实测中提出四项诉求，现状各自都有可指认的缺陷：

1. **没有滚动条**：消息区只靠滚轮/`PgUp`/`PgDn` 改变 `scroll_offset`，视口在长会话里
   「不知道自己处在历史的哪一段」。`scroll_total` / `scroll_offset` / 可视高度三个量在
   `ui()` 里已经齐备，缺的只是把它们映射成最右列的几何符号。
2. **思考会占满视口**：ADR 0076 让 `MessageRole::Thinking` 逐行渲染，把超长 reasoning 全部铺开
   （0076 已登记为「已知残余 ①：思考块不做折叠/截断」）。一轮长思考之后正文被推出视口，
   用户要手工滚回去才能读到回答。
3. **没有设置入口**：TUI 里所有显示形态都是编译期常量或临时按键，用户无法持久化自己的偏好；
   `/models`、`/reasoning` 已证明「斜杠命令 + 面板」是 TUI 里成本最低、发现性最好的入口形态，
   但显示偏好没有对应面板。
4. **Agent 过程噪声**：轮次之间的工具调用 / 工具结果 / 思考块把对话切得很碎，
   用户希望「只看对话与状态消息」，并且**默认为隐藏**；隐藏期间仍需知道「现在在跑哪个工具」。

## 候选方案与拒绝理由

- **A. 滚动条画进正文列（右侧 padding）**：拒绝——正文折行宽度会随之变化，既有布局断言
  （`tui_render_layout`）与所有折行黄金值连锁失效；滚动条必须占用**独立列**。
- **B. 把显示偏好塞进桌面端 `settings.json` / `providers.json`**：拒绝——那是跨端契约文件
  （前端类型定义 + 迁移链），TUI 的两个渲染开关混写进去会污染桌面端 schema，
  也会让「终端偏好」随桌面端迁移一起被改写。
- **C. 全局快捷键「展开一切」而不持久化**：拒绝——用户诉求包含「可开关、默认为隐藏」，
  不持久化意味着每次启动都要重新按；且「展开一切」把思考折叠与过程隐藏混成一个状态，
  两个维度无法独立表达（要折叠但不要隐藏是合法组合）。
- **D. 隐藏时直接从 `messages` 里删掉思考/工具消息**：拒绝——破坏会话回放、`/resume` 与
  鼠标选区几何；隐藏必须是**渲染层过滤**，数据一律保留。
- **E. 折叠时丢弃思考原文**：拒绝——同上，折叠只是**单行摘要 + 原文仍在内存/历史**。
- **F. 采用方案**：独立最右列滚动条（纯函数几何）+ 思考块单行折叠 + `InteractionMode::Settings`
  面板 + 渲染层过程过滤，偏好落独立文件 `tui-preferences.json`。

## 决策

1. **滚动条几何纯函数化**：`scrollbar_thumb(total, visible, offset) -> Option<(u16, u16)>`
   ——`visible == 0` 或 `total <= visible` ⇒ `None`（不画）；否则
   `thumb_len = max(1, round(visible²/total))`，
   `start = round(offset * (visible - thumb_len) / max(1, total - visible))`，
   `start + thumb_len <= visible` 饱和截断。几何不进渲染函数，先单测后上屏。
2. **滚动条渲染位置**：终端**最右列**（`area.x + area.width - 1`），纵向严格限制在
   `chat_rect` 内；轨道 `│`（`theme::GHOST`）、滑块 `█`（`theme::ACCENT_DIM`）；
   正文可用宽度不变（不侵占、不越界）。溢出才画，空态不画。
3. **思考折叠**：`thinking_collapsed == true` 时 `MessageRole::Thinking` 块只产出一行
   `✻ 思考 · {n} 行 · 已折叠`（`n = content.lines().count()`，空内容为 `0`；
   `✻ ` 用 `ACCENT_DIM`，后半段用 `FAINT`），块尾保留 1 行呼吸（ADR 0076 的既有间距契约不变）。
   展开态与 ADR 0076 逐字一致。
4. **`/settings` 面板**：新增 `InteractionMode::Settings`；`/settings` 进入 `AVAILABLE_COMMANDS`
   并由 `is_panel_command` 判定为面板命令（**流式期同样可开**，不排队、不拒绝）。
   面板行由 `settings_rows()` 单一真值派生（`hide_process`、`thinking_collapsed`，固定顺序），
   `↑↓` 选择（首尾不环绕）、`Space`/`Enter` 切换、`Esc` 关闭回 `Normal`。
5. **过程隐藏**：`hide_process == true` 时 `Thinking` / `ToolCall` / `ToolResult` 三类块的渲染行数为 0，
   `User` / `Assistant` / `System` 不受影响；判据收敛为纯函数 `should_hide_message(role, hide_process)`，
   过滤点位于 `message_lines_with_ctx` 顶部（**渲染层早退，数据不删**）。
6. **隐藏态的状态消息**：`is_streaming && hide_process && runtime_mode == ExecutingTool(_)` 时，
   消息区末尾状态行照常渲染，文案 `执行 · <工具名>[ · <摘要>]`（取自最近一条 `ToolCall`，
   按可用宽度截断），由 `hidden_tool_status_text` 单点产出；`stopping` 分支优先级不变。
7. **偏好持久化独立成文件**：新增模块 `agent::tui_preferences`，
   路径 `<config_dir>/pony-agent/tui-preferences.json`（与 `model-preferences.json` 同目录、
   独立文件），`camelCase` 字段、缺失字段一律回落产品默认、原子写（`*.json.tmp` → rename）；
   读取失败/损坏静默回落默认值，绝不 panic。
8. **状态机不写盘**：`set_hide_process` / `set_thinking_collapsed` / `toggle_selected_setting`
   只改内存并 `invalidate_render_cache()`；持久化由事件循环在切换后统一调用
   `TuiPreferencesStore::save`（面板与 `Ctrl+O` 共用同一写入点）。
9. **默认值口径（R1）**：`TuiPreferences::default()` = 隐藏 + 折叠（产品默认），
   `run_tui` 启动时经 `state.apply_tui_preferences(&store.load())` 落地；
   `TuiState::default()` 保持**旧行为**（可见 + 展开，不做 IO），沿用 `model_preferences`
   「Default 可复现、不做 IO」的约定，以免破坏已冻结的 `TuiState::default()` 断言。
10. **`Ctrl+O` 快捷开关**：`InteractionMode::Normal` 下切换 `hide_process` 并落盘，
    状态栏给一次性提示（`set_status_notice`）；与 `/settings` **读写同一字段**，
    不存在第二份真值。

## 影响

- **代码面**：`crates/pony-agent-core/src/bin/tui.rs`（滚动条几何/渲染、`thinking_fold_label`、
  `should_hide_message`、`hidden_tool_status_text`、`MessageRenderCtx` 两字段、
  `InteractionMode::Settings` 与面板状态机、`Ctrl+O`、缓存失效判据）；
  新增 `crates/pony-agent-core/src/agent/tui_preferences.rs`（+ `agent/mod.rs` 注册）。
  `message_lines_for` 旧入口保持旧行为（两开关均 `false`），既有虚拟化/布局测试的参考路径不变。
- **纯本地 UI**：无网络 IO、无 LLM 调用、无后台任务、无对外接口、无持久化 schema 变更
  （只多一个独立偏好文件）；可逆面 = 删除偏好文件即回落产品默认。
- **既有契约**：ADR 0076 的思考渲染与底部呼吸、ADR 0042 的 composer 几何、ADR 0038 的层级、
  ADR 0071 的流式期输入/面板语义均未被改写；`/settings` 复用 0071 的面板命令裁决路径。
- **已知残余（本轮明确不处理）**：① 隐藏态下复制/选区的纯文本面只覆盖可见行
  （`rendered_lines_text` 与渲染缓存同源，隐藏内容不可复制；数据本身未删，展开后仍在）；
  ② 隐藏态状态行的工具信息取「最近一条 `ToolCall`」，新轮工具尚未落消息的瞬间可能显示上一轮工具；
  ③ 滚动条只覆盖终端最右列——超宽终端正文居中限宽 120 时两者距离较远（契约 C2 的设计选择）。
- **§3 观测性**：结构性 N-A（见 `docs/observability.yaml` 的 wave-tui-process-fold 归属登记），
  无新增收口点，不伪造 `/metrics`。

## 验收

契约矩阵 `.dev-team/contract-matrix-tui-process-fold-scroll-settings.md` 的 C1–C12：

| 契约 | 覆盖测试（`crates/pony-agent-core/tests/tui_process_fold_acceptance.rs`） |
| :-- | :-- |
| C1 | A1（`scrollbar_thumb` 表驱动） |
| C2 / C3 | A2 / A3（TestBackend 逐格断言最右列与空态） |
| C4 / C5 | A4 / A5（折叠行文案 + 展开态逐字回归） |
| C6 | A6（命令表 + `is_panel_command` + 流式期可开） |
| C7 / C8 | A7 / A8 / A9（面板状态机、切换不写盘、store 单元） |
| C9 | A10 / A11（三类块 0 行；User/Assistant/System 不受影响） |
| C10 | A12（隐藏态状态行含 `执行 · ` 与工具名，且无工具行字形） |
| C11 | A13 + A9（`TuiPreferences::default()` 隐藏+折叠；`TuiState::default()` 仍可见+展开） |
| C12 | A14（`Ctrl+O` 切换 + 落盘 + 一次性提示，与面板同字段） |

回归：`tui_thinking_bottom_gap.rs`（ADR 0076 的 A1–A14 冻结断言）、`tui_render_layout.rs`、
`tui_batch6_virtualization.rs`（`message_lines_for` 旧行为）、`tui_terminal_restore.rs`。

## 关联

- 承接 ADR 0076（思考消息可见与底部呼吸）登记的残余 ①「思考块不做折叠」；
- 复用 ADR 0071 的「流式期面板可开 + 面板命令裁决」与 ADR 0033 的 composer/状态行几何；
- 与桌面端 `settings.json`（前端契约）**解耦**：同一语义在不同交付形态各自表达，不共享文件。
