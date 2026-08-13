# workspace-sidebar-tree-navigation

## Background

`HomeSessionSidebar.vue` 目前平铺展示全部会话（`sessionList` 全局列表 + 当前会话插头），无项目分组。Workspace 功能要求在侧边栏按"项目（二级目录）→ 对话（三级目录）"呈现：项目层级折叠，展开后是该 Workspace 下发生的对话。数据基础由 PA-079（`SessionOverview.workspaceId` + workspace 注册表）提供。

3 路对抗审核（2026-08-09）确认的关键约束：

- **激活 workspace 的单一真相源 = 前端 localStorage**：PA-079 已移除后端 `workspace_activate`，本卡拥有激活持久化（`pony-agent.active-workspace.v1`）；后端 `workspace_list` 只提供注册表数据，不提供激活。
- **瞬态"新对话"条目必须带 `workspaceId`**：侧边栏注入的未持久化当前会话条目（`HomeSessionSidebar.vue:83-91`）没有 `workspaceId`，若按 `?? "default"` 归组会落在默认组——激活 workspace ≠ default 时"新建对话"落错组。合成条目须取 `workspaceId = activeWorkspaceId ?? "default"`。
- **AC2 原措辞自相矛盾**：树形设计展示全部 workspace 组，不存在"切换后列表过滤"；改语义为"切换激活 workspace 改变新会话创建目标，但不隐藏其他组"。
- **分页 × 分组交互未定义**：现有"显示更多"作用于平铺列表（`HomeSessionSidebar.vue:96-98,152-157`）；改为分组树后需明确"显示全部"作用于所有组。
- **孤儿/过期状态需防崩溃**：注册表丢失记录的 `workspaceId`、localStorage 残留的折叠状态、浏览器模式无宿主，都需有明确降级（"未分组"组、忽略未知 key、默认组）。

## Goals

- 侧边栏会话列表重构为两级树：二级 = Workspace（项目），三级 = 该 Workspace 下的对话。
- 支持组折叠/展开（状态本地持久化）、当前会话自动归组、Workspace 管理入口（新建/切换）。
- 空 workspace 有明确空态；与既有会话操作（切换/删除/新建）行为保持一致。
- 激活 workspace 单一真相源（localStorage）；新建对话默认归属激活 workspace。

## Non-goals

- 不做会话拖拽跨 workspace 移动（后续迭代）。
- 不做 workspace 内文件浏览器（后续候选）。
- 不改变会话历史/分支/恢复行为。
- 不引入新的路由体系（保持现有 page 切换机制）。
- 不做授权审批 UI（PA-080 提供 API 面）。
- 不做分页的 per-group 细分（"显示全部"作用于全部组；per-group 分页后续候选）。

## Scope

- `src/components/HomeSessionSidebar.vue`：树形分组渲染（workspace 组 + 会话项）、折叠/展开、空态、孤儿"未分组"组、瞬态条目带 workspaceId。
- `src/stores/runtime.ts`：`workspaceList` / `activeWorkspaceId` 状态、`createWorkspace` / `activateWorkspace` action（`createWorkspace` 调 PA-079 host command；`activateWorkspace` 仅写 localStorage）、`createSession` 携带 active workspace。
- 持久化 key：`pony-agent.session-sidebar-workspace-groups.v1`（折叠状态，对齐既有 `pony-agent.session-sidebar-*` 前缀）、`pony-agent.active-workspace.v1`（激活）。
- 前端单测：分组/折叠/切换/归组/空态/孤儿/浏览器模式。

## Risks

- 会话与 workspace 归属不一致（旧数据 None）：前端按 `workspaceId ?? "default"` 归组，与后端投影一致；"default" 常量与 PA-079 共享（单源导出，跨端测试断言一致）。
- 大量会话性能：分组用 computed 缓存（一次分组，不逐次 filter）。
- 折叠状态与激活 workspace 冲突：激活 workspace 组默认展开；手动折叠仍持久化（激活组展开只在本轮激活时生效，不覆盖持久化的手动折叠值？——见设计：以持久化值为准，激活时仅当无持久化值才默认展开）。
- 孤儿 workspaceId：注册表无该记录 → 归入"未知工作区/未分组"组，不崩溃。
- 浏览器模式：无宿主 → 仅显示默认组，管理入口隐藏或禁用，避免 host command 失败噪音。
- 测试口径：持久化测试为 jsdom 下 write→remount-read（既有 `HomeSessionSidebar.spec.ts:620-636` 模式已证明可行）；真正的进程重启为 e2e 范畴，不在单测承诺内。

## Validation

- 前端 vitest（新增/更新 HomeSessionSidebar 相关测试：分组正确性含 None→default、折叠持久化、激活切换、空态、新建 workspace、孤儿组、浏览器模式、瞬态条目归组）、`npm run build` 通过。
- 既有会话侧边栏测试更新后全绿。
