# Agent Note: TUI `/resume` 列表重排与会话归档（跨区前缀 / 隐藏归档 / archive 命令与快捷键）

Status: implemented

## Problem

TUI 的 `/resume`（`crates/pony-agent-core/src/bin/tui.rs` 的 `/resume` 分支与
`InteractionMode::SelectingSession` 渲染）有三处真实缺口：

1. **右列信息无价值**：每行右侧是 `{turn_count} 轮 · {时间} · {短id}`。轮次对"选哪个会话"
   零帮助，短 id 只在极少数"抄 `pony resume <id>`"场景有用，却挤掉了标题宽度。
2. **跨工作区无标识**：`list_sessions()` 全量投影里既有当前目录的会话，也有其他 workspace 的
   会话（`SessionOverview.workspace_id`）。选中跨区会话会把 `active_workspace_id` / 工具根
   悄悄换成目标会话归属（`hydrate_state_from_session` → `resolve_resumed_workspace_binding`），
   用户以为还在原目录——列表里没有任何先行提示。
3. **归档在 TUI 不可达**：数据面早有 `SessionState.archived`（桌面侧栏用它隐藏会话、
   `control_plane.archive_session/unarchive_session` 已存在），但 TUI 既不过滤也不提供归档入口，
   历史会话只增不减；`/resume` 空态还只说"暂无历史会话"，不区分"这个工作区没有"与"全都没有"。

## Decision

### 1. 行格式：右列只留时间，跨区前缀"差额显示"

`bin/tui/resume_list.rs::resume_row_parts` 返回 `(left, right)` 两段：

- `right` 首个 token = 时间（`format_session_row_time` 既有口径）；仅 `is_active` 追加
  ` · 当前`，仅 `is_selected` 追加 ` · <短id>`。**轮次列删除**。
- 跨区前缀 `"<label> · "` **仅在** `workspace_id != current_workspace_id` 且两者都非空时出现，
  属于 `left`。当前区行零额外字段——这正是"信息过多"的解：不是人人加徽标，而是只给少数
  需要警示的行加。
- `str_width(left) + 1 + str_width(right) ≤ width`；宽字符按 2 列、溢出 `…` 收尾；`width` 取
  `min(96, 终端宽-4)`。任何宽度（含 20 列）与超长标题不得 panic / 越界。

### 2. 可见性：默认隐藏归档，空态计数不撒谎

`resume_list.rs::session_visible` = `is_main_session_id && !continuation_only &&
(show_archived || !archived)`，过滤只减行不改序（输入序即 `updatedAt desc`）。面板内 `a`
切换 `show_archived`，`d` 归档高亮行、`u` 取消归档。

空态文案固定为 `暂无历史会话（当前工作区 {cur} 条 · 全部 {all} 条）`；有归档被隐藏时追加
`（归档 {h} 条已隐藏 · 按 a 显示）`。三条数字同源于 `list_sessions()` 数据面，避免 ADR 0094
那次"索引缺失被读成没有历史"的谎报形态。

### 3. 跨区切换提示：事实驱动，不预测

`resume_list.rs::workspace_switch_notice(old, new, label, root)` 仅在 `old != new` 且 `new`
有效时返回 `Some("已切换到工作区 <label>（<root>）")`。调用点必须在 `hydrate_state_from_session`
**之后**比对切换前后的 `active_workspace_id`——提示的是已发生的事实，不是"我猜它跨区了"。

### 4. 归档：`/archive` 三态 + `Ctrl+X`，复用既有数据面

`resume_list.rs::archive_plan(is_streaming, turn_in_flight, arg)` 三态：

| 态 | 条件 | 行为 |
|---|---|---|
| `BlockedDuringStreaming` | 流式中或轮次在飞 | 拒绝并在状态行说明，不改任何状态 |
| `ArchiveOther(id)` | 未阻塞且 `arg.trim()` 非空 | 只归档指定 id，不切会话 |
| `ArchiveCurrentThenRotate` | 未阻塞且无参 | 归档当前会话后**换新 session_id**（与 `/clear` 同法：`new_session_id()` + `reset_session_scope()` + goal 续轮登记迁移） |

- 短接键 `Ctrl+X` = 无参 `/archive`，仅在 Normal（输入框）模式生效；`SelectingSession` 面板内
  不复用，面板收起归档走 `d`/`u`——避免同一键在两种模态下语义漂移。
- 归档 ≠ 清空：消息与历史保留，`pony resume <id>` 仍可恢复；`/resume` 内 `a` 可再看见它。
- 归档/取消归档后打既有 `tui_queue_log` 结构化行，不新增遥测出口。

> 标题取值沿用三线投影（用户改名 > AI 标题 > 派生），见
> [`2026-11-15-session-title-three-track.md`](2026-11-15-session-title-three-track.md)；本条只改
> 列表的**布局与可见性**，不改标题生成。

## Alternatives considered

- **按当前工作区硬过滤列表**（用户最初的方案）：会隐藏"别的目录的历史会话"，而恢复它们本来
  就是允许的（权限跟会话归属走、不是跟启动目录走），过滤只换来"看不见"却留下"看不到就打不开"
  的死角；且早年未盖章会话（`workspace_id=None` → default）会整批消失。改为"保留全量 + 跨区
  前缀 + 切换提示"，把风险移到用户可判断的位置。
- **按活跃度只保留最近 X 条**：现状已是 `updatedAt desc` + 虚拟滚动窗口（`panel_window`），
  老会话自然沉到窗口外但仍可滚到。硬截断会让最需要 `pony resume <id>` 的冷门会话永久不可达，
  且与"归档"语义重复——归档是用户主动收起，按时间自动消失则不可预期，形同静默删除。
- **TUI 侧新增独立归档标记/独立的"已归档视图"**：数据面 `archived` 已贯通（blob、规范化表、
  `list_sessions` 投影、`archive_session`/`unarchive_session` 与桌面共用），另起字段只会让两侧
  口径分叉。
- **`Ctrl+B` / `Ctrl+D` 作为归档快捷键**：`Ctrl+B` 是 tmux 前缀（TUI 常在 tmux 里跑），
  `Ctrl+D` 是 EOF 肌肉记忆，二者承载可逆但"会话被收起"的动作都会造成误触；取 `Ctrl+X`。
- **桌面侧同步改列表口径**：桌面是全局多工作区树（`deriveSidebarTree` 按工作区分组、可切
  `showArchived`），与本面的"单目录 TUI"语义不同；同步改会把两种场景的约束强行归一。

## Consequences

- 收益：`/resume` 行宽还给标题，跨区切换可被看见、可被回溯；历史会话有了主动收起的出口，
  且与桌面共用同一 `archived` 事实；空态不再谎报"没有历史"。
- 代价：`SessionItem` 增三个字段（`workspace_id` / `workspace_label` / `archived`），
  `/resume` 渲染与键位分支变更，`list_workspaces()` 在打开面板时被读一次（本地注册表，无 IO 放大）。
- 守护：契约真源 `.dev-team/contract-matrix-tui-resume-archive.md`（C1–C6 / T1–T8）由独立
  Test Agent 冻结为 `tests/tui_resume_archive_acceptance.rs`（含对抗用例），实现侧只读。
