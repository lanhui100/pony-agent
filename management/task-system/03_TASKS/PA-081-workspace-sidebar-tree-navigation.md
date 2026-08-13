# PA-081 侧边栏 Workspace 树导航

## Basic Info

- ID: PA-081
- Status: Ready
- Priority: P1
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-08
- Updated At: 2026-08-08
- OpenSpec Change: `workspace-sidebar-tree-navigation`（3 路对抗审核已通过，2026-08-09）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订）

## Background

`HomeSessionSidebar.vue` 目前平铺展示全部会话，无项目分组。Workspace 功能要求在侧边栏按"项目（二级目录）→ 对话（三级目录）"呈现：项目层级折叠，展开后是该 Workspace 下发生的对话。

## Goal

将侧边栏会话列表重构为两级树：二级 = Workspace（项目），三级 = 该 Workspace 下的对话；支持折叠/展开、会话归属过滤、新建/切换 Workspace 入口；与 PA-079 数据模型、PA-080 权限边界保持一致呈现。

## Scope

- 前端：`HomeSessionSidebar.vue` 树形结构改造（项目分组 + 折叠/展开）
- 会话按 workspaceId 分组展示；当前会话自动归组
- Workspace 管理入口（新建/切换），调用 PA-079 注册表 API
- 空态/加载态/权限提示（读 workspace 外文件时的审批反馈）
- 前端单测 + vitest + 构建验证

## Non-Goals

- 不做会话拖拽移动跨 workspace（后续迭代）
- 不做 workspace 内文件浏览器（后续候选）
- 不改变会话历史/分支行为
- 不引入新的路由体系（保持现有 page 切换机制）

## Acceptance Criteria

1. 侧边栏按 workspace 分组展示会话，组可折叠/展开，状态本地持久化。
2. 新建对话默认归属当前激活 workspace；切换激活 workspace 只改变新会话创建目标，不隐藏其他组（树形展示全部组）。
3. 当前会话不在激活 workspace 时，自动选中其归属组。
4. 空 workspace 有明确空态提示。
5. 前端 vitest、`npm run build` 全绿；既有会话侧边栏测试更新后通过。

## Review Plan

- @consultant：导航信息架构、与现有侧边栏交互模型的兼容
- @code-reviewer：store 状态管理、分组计算性能、持久化 key 规范
- @tester：折叠/切换/归组边界用例

## Current Progress

- 3 路对抗审核（2026-08-09）已完成，findings 全部采纳；spec/proposal/design/tasks 已修订（激活 localStorage 单一真相源、AC 语义改写"切换不隐藏组"、瞬态条目带 workspaceId、孤儿"未分组"、"显示全部"解除组上限等）。详见 `02_REVIEWS/2026-08-09-pa078-081-spec-review.md`。
- 无未解决 P0/P1，spec 通过审核，可进入实现。

## Next Action

- 按 `openspec/changes/workspace-sidebar-tree-navigation/tasks.md` 顺序实现（依赖 PA-079 数据模型与 PA-080 权限反馈语义）。

## Blockers

- 依赖 PA-079（数据模型）与 PA-080（权限反馈语义）

## Resume Hint

- 先读 `src/components/HomeSessionSidebar.vue`、`src/stores/runtime.ts` 的 sessionList/sessionId、`src/lib/runtime/sessions.ts`
