# 契约矩阵：TUI `/resume` 列表重排 + 会话归档（C1~C6）

> 状态：**frozen**（变更须 Lead 原地改写 + 文末流水）｜任务：shared task-1（T1/Protocol）｜基线分支 `feat/tui-resume-archive`
> 唯一真源：本文件。代码侧只落**纯函数空桩**（函数体 = `todo!()`），实现归 Executor，验收测试归 L2-T/AT。
> 落点：`crates/pony-agent-core/src/bin/tui/resume_list.rs`（C1~C5）、`.../tui/input.rs`（C6 键位）、`.../bin/tui.rs`（SessionItem 字段 / `AVAILABLE_COMMANDS` / 渲染接线）。

## C1 行格式（`resume_row_parts(input, width) -> ResumeRowParts`）

一行 = `[跨区前缀] 标题 ……… 时间[ · 当前][ · <短id>]`，左列 `left`、右列 `right` 分离返回。

- **右列 = 时间**：`right` 的首个 token **必须**是 `updated_at`（已格式化：`HH:MM` / `MM-DD HH:MM` / `YYYY-MM-DD` / 缺失回退 `—`）。
  仅当 `is_active` 追加 `" · 当前"`；仅当 `is_selected` 追加 `" · <短id>"`。二者同时在位 → `时间 · 当前 · <短id>`（顺序固定）。
- **短 id**：`session_id.rsplit('-').next()`（无 `-` 时取整串）再取**前 8 个 char**（按 char 计，不按字节）。
- **轮次禁止出现**：右列**不得**含 `轮` 或 `N 轮` 片段（旧实现 `{turn_count} 轮 · {time} · {id}` 作废）。
- **跨区前缀**：仅当 `workspace_id` 与 `current_workspace_id` **都非空**且**不相等**时出现，形态 `<label> · `；
  `label` = `workspace_label`（去空白后非空）否则回落 `workspace_id`。同区 / 任一为 `None` / 任一为空白 ⇒ 无前缀。
  前缀属于 `left`（左列）的一部分。
- **截断**：`left = ellipsize_width(prefix + title, left_budget)`，`right = ellipsize_width(right, right_budget)`，
  其中 `right_budget = width.saturating_sub(2)`、`left_budget = width.saturating_sub(str_width(right) + 1)`。
  溢出统一以 `…` 收尾（`ellipsize_width` 语义：宽字符按 2 列计）。
- **不变量**：`str_width(left) + 1 + str_width(right) ≤ width`（`right` 非空时）；`width == 0` ⇒ 两者均空；
  任何 `width`（含 1、20 列窄终端）与任何超长标题（>200 字）**不得 panic、不得越界**。
- 标题中的用户正文可含任意字符（含 `轮`、`·`）；T1 断言作用域 = 右列 + 左列结构，**不**对标题做内容断言。

## C2 可见性（`session_visible(session_id, archived, continuation_only, show_archived) -> bool`）

```
is_main_session_id(session_id) && !continuation_only && (show_archived || !archived)
```

- `is_main_session_id`：排除 `subagent-*` / `teammate-*` 前缀（core `agent::session::id` 既有口径；空 id 视为主会话）。
- `continuation_only`：仅由续轮提示构成的碎片会话不进 `/resume`。
- `archived`：默认隐藏；`show_archived == true` 时才可见（面板内 `a` 切换，见 C3/C6）。
- 顺序：过滤**只减行不改序**——不排序、不分组、不去重；输入序即 `updatedAt desc`（宿主 `list_sessions()` 保证）。
- 数据面（C3 计数用）与可见面（本函数）是两个口径，不得混用。

## C3 空态计数（渲染层，非纯函数）

- 标题：`会话 · 共 {N} 条`，`N` = **当前可见行数**（C2 过滤后）；当 `N > shown`（一屏可显示行数）追加 `（{sel+1}/{N}）`。
- 空态（`N == 0`）正文首段固定：`暂无历史会话（当前工作区 {cur} 条 · 全部 {all} 条）`。
  - `all` = 数据面总行数（`list_sessions()` 原始行，含归档 / 续轮碎片 / 队友会话）；
  - `cur` = 其中 `workspace_id == current_workspace_id` 的行数（同一数据面口径）。
- 不撒谎：`show_archived == false` 且被隐藏的归档行数 `h > 0` 时，空态追加 `（归档 {h} 条已隐藏 · 按 a 显示）`。
- 非空但可见为 0 只会发生在「全部是归档行」时，同样走空态文案（不得显示「共 0 条」却仍列行）。

## C4 跨区提示（`workspace_switch_notice(old_id, new_id, label, root_path) -> Option<String>`）

- 仅当 `old_id != new_id` **且** `new_id` 为 `Some(非空)` ⇒ `Some("已切换到工作区 <label>（<root_path>）")`；否则 `None`。
- `<label>` = `label`（去空白后非空）否则回落 `new_id`；`root_path` 原样内嵌（不校验、不转义、不 panic）。
- `old_id == None`（无历史工作区）视为**不相等**，只要 `new_id` 有效即返回 `Some`。

## C5 归档动作（`archive_plan(is_streaming, turn_in_flight, arg: Option<&str>) -> ArchivePlan`）

三态，判定顺序即优先级：

1. `BlockedDuringStreaming`：`is_streaming || turn_in_flight` —— **任何参数都拒绝**（参数不参与判定，`/archive <id>` 同样被拦）。
2. `ArchiveOther(String)`：未阻塞且 `arg.trim()` 非空 ⇒ 携带 `trim()` 后的目标 id。
3. `ArchiveCurrentThenRotate`：未阻塞且 `arg` 为 `None` 或全空白。

- 无参归档当前会话后**必须换新 session_id**：与 `/clear` 同法——`new_session_id()` + `reset_session_scope()`，
  并同步 goal 续轮取消登记（`cancel_for(old)` → `reset_for(new)`）。归档 ≠ 清空：消息与历史保留，只是换 id 开新会话。
- `ArchiveOther` 只归档指定 id，**不**换当前 session_id、**不**复位作用域。
- 阻塞态必须给用户可见回执（状态行），不得静默吞掉。

## C6 键位 / 命令

- `is_archive_shortcut(code, modifiers) -> bool`：只认「**恰好** `CONTROL` + `Char('x')`」——`modifiers == KeyModifiers::CONTROL`（严格相等，非 `contains`）。
  不命中：`Ctrl+Shift+X`（`Char('X')` 或带 SHIFT 位）、`Alt+X`、裸 `x`、`Ctrl+Alt+X`、`KeyCode::Char('X')`。
- 生效范围（Lead 裁决）：**Normal 模式（输入框态）全局可用**，语义 = 无参归档当前会话（`archive_plan(.., None)` 全路径）；
  `InteractionMode::SelectingSession` 面板内 `Ctrl+X` **不生效**——同一快捷键不在两种模态下漂移语义。
- 面板内归档/取消归档走 `d` / `u`，显示归档走 `a`（`show_archived` 默认 `false`）；
  与既有 SelectingSession 键位（`Esc`/`↑`/`↓`/`Enter`）无冲突。
- `AVAILABLE_COMMANDS` 追加且只追加这一条：`("/archive", "归档当前会话（/archive <id> 归档指定会话）")`；`/help` 逐行渲染时必须能显示它。
- `/archive` 在命令分发侧必须**先于**整串匹配的 `known_command` 处理分支（与 `/goal` 同因：允许带参数）。

## 断言映射（验收测试唯一对照表）

| # | 断言主题 | 契约 | 断言点（可机械判定） |
|---|---|---|---|
| T1 | 行格式 | C1 | 右列首 token == 时间；右列不含 `轮`；左列不含时间；`active`/`selected` 标记按条件出现且顺序固定；短 id == `-` 末段前 8 char；跨区前缀条件真值表；`str_width(left)+1+str_width(right) ≤ width`（width ∈ {0,1,20,40,84,200}） |
| T2 | 跨区前缀条件 | C1 | 4 组：两个 id 都非空且不等 ⇒ 有；相等 ⇒ 无；任一 `None` ⇒ 无；`workspace_label` 缺失/空白 ⇒ 回落 `workspace_id` |
| T3 | 归档可见性 | C2 | 8 组真值：`(is_main × continuation_only × archived × show_archived)`；`subagent-`/`teammate-` 恒 false；过滤保持输入序 |
| T4 | 空态计数 | C3 | 标题含 `共 {N} 条`；空态文案同时含 `当前工作区 {cur} 条` 与 `全部 {all} 条`；归档全隐藏时含 `归档 {h} 条已隐藏` 且 `N == cur == all == 0` 时数字自洽 |
| T5 | 跨区通知真值表 | C4 | `(old,new)` 4×3 组（None/相同/不同/空串）→ `None`/`Some(精确串)`；`label` 缺失回落 `new_id` |
| T6 | 归档三态 | C5 | `streaming`/`turn_in_flight` × `arg` ∈ {None, "", "  ", "abc", " abc "} → `BlockedDuringStreaming` 优先；其余按 trim 分流；`ArchiveOther` 携带 trim 后 id |
| T7 | 键位矩阵 | C6 | 纯函数：`Ctrl+X` 命中；`Ctrl+Shift+X`/`Alt+X`/裸 `x`/`Ctrl+Alt+X`/`Shift+X` 不命中。接线断言（review/渲染级）：`Ctrl+X` 只在 Normal 分支派发，面板内不得复用 |
| T8 | 命令表 | C6 | `AVAILABLE_COMMANDS` 含且仅含一条 `/archive` 条目（描述逐字相等）；`/help` 渲染行含 `/archive` |

## Lead 裁决修订（R1~R10，2026-11-21，优先于上文对应条款）

- **R1（改 C1）**：`resume_row_parts` 的 `width` = 面板**正文内宽** `body.width`，不是面板外宽；面板宽 `min(96, area.width-4)`。
- **R2（改 C1）**：不得复用 `list_row`（其右列预算 8 列、`truncate_width` 不补 `…`）；渲染层自建两列行，left 用补 `…` 的 ellipsize。
- **R3（改 C3）**：`all`/`cur`/`h` 三个计数在 `/resume` 分支由**过滤前 raw 列表**算出并存入 `TuiState`；**禁止**给 `SessionItem` 增字段（冻结测试的结构体字面量无 `..`）。
- **R4（补 C2 调用面）**：`state.session_list` 只存 C2 过滤后的视图；渲染 / Enter / `d` / `u` 共用同一索引并 clamp。
- **R5（补 C5）**：无参归档换会话必须抄全 `/clear` 四步：`pending_request=None` + `close_pending_request_overlay` → `goal_round_cancel.cancel_for(old)` → `new_session_id()` + `reset_for(new)` + `reset_session_scope()` → `sync_pending_control_request`；并回执 `已归档 <id>，可用 pony resume <id> 找回`。
- **R6（补 C6）**：面板 `d`/`u` 只对**非当前**会话行生效；当前行按 `d`/`u` 只给提示「当前会话请用 /archive 或 Ctrl+X」；`archive_session/unarchive_session` 的 `Err` 必须进状态行。
- **R7（补 C6）**：处于队友/子会话时，`Ctrl+X` 与无参 `/archive` 拒绝并提示先返回主会话。
- **R8（改 C1 渲染补充）**：归档行只 dim，不加 `· 归档` 后缀；`a` 显示归档时标题追加 `· 含归档`。
- **R9（钉 C4 调用点）**：toast 唯一调用点 = `SelectingSession` Enter 恢复分支（hydrate 后比对前后 `active_workspace_id`）；禁接 `switch_session_scope` 与 `hydrate_state_from_session`。
- **R10**：归档成功/失败一律可见回执，`let _ =` 视为未完成。

## 非目标（Out of scope）

- 不改 `SessionOverview` / 会话后端 schema / 归档持久化语义（`archived` 已在数据面）。
- 不新增侧边栏、不改桌面端；不做多工作区并行调度。
- 不改 `/resume` 的 Enter 语义（仍是 `switch_session_scope`）。

## 流水

- 2026-11-21：Protocol Agent 冻结 C1~C6 + T1~T8，落 `todo!()` 空桩（`target = feat/tui-resume-archive`）。
- 2026-11-21：Lead 裁决修订 C6 生效范围——Ctrl+X 改为 **Normal 模式（输入框态）全局无参归档**，
  `SelectingSession` 面板内不生效（面板内 `d`/`u` 归档/取消归档、`a` 显示归档）；T7 增「接线断言」一句。C1~C5 未动。
- 2026-11-21：L3 方案审查 6 条 BLOCKING → Lead 裁决 R1~R10（宽度绑定/不复用 list_row/计数通道/索引空间/Ask 收口/面板 d-u 语义/子会话守卫/归档行样式/toast 调用点/回执）。
