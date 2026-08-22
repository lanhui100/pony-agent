# PA-081 侧边栏 Workspace 树导航

## Basic Info

- ID: PA-081
- Status: Done
- Priority: P1
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-08
- Updated At: 2026-08-22
- OpenSpec Change: `openspec/changes/archive/2026-08-22-workspace-sidebar-tree-navigation/`（2026-08-22 归档；canonical spec 落于 `openspec/specs/workspace-sidebar-tree/spec.md`）
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
- **2026-08-22 实现完成**：
  - **store**：`workspaceList`/`activeWorkspaceId`/`workspaceListLoaded` 状态；`loadWorkspaces`（幂等 + contained 失败降级）/`normalizeActiveWorkspace`（激活项不在注册表→回退 default 并清理残留 key）/`createNewWorkspace`（成功自动激活）/`activateWorkspace`（localStorage 单一真相源 `pony-agent.active-workspace.v1`）；瞬态"新对话"与 TurnInput 均携带 `activeWorkspaceId ?? default`。
  - **分组纯函数** `src/lib/runtime/sidebar-groups.ts`：None→default、命中注册表→对应组、未知 id→尾部"未分组"、空组保留（空态面）、折叠持久化读写（`pony-agent.session-sidebar-workspace-groups.v1`，读取忽略未知 key、写入按当前合法组裁剪）。
  - **组件** `HomeSessionSidebar.vue`：扁平行模型（group-header/session/group-empty）单份会话行模板复用；组头名称+全量计数徽标+折叠；"显示全部"一键解除所有组预览上限（每组前 5）；Workspace 管理节（列表/切换/新建表单，浏览器模式禁用 + 提示）；空态"暂无对话"。
  - **测试**：新增 `tests/sidebar-groups.spec.ts`（7 项纯函数契约：None→default/合成默认组/孤儿尾置/空组保留/瞬态归激活组/折叠往返/损坏容错）；扩展 `HomeSessionSidebar.spec.ts` 7 项组件用例（分组顺序/瞬态归组/折叠持久化+过期 key 忽略/切换不隐藏组/新建自动激活/浏览器模式/空态）；`runtime-store.spec.ts` 瞬态期望同步。
- **2026-08-22 实施后双路对抗审核（正确性回归 / spec-design 符合度，均"有条件通过"）——采纳修复全部落地**：
  - **P1-1 自动展开**：当前会话所在持久化折叠组自动展开（本 boot 显式折叠优先，`bootToggledGroupKeys` 守卫）+ 组件用例锁定。
  - **P1-2 跨项目落点**：submitTurn 改以**会话自身归属优先**（`sessionWorkspaceId` 冻结于创建时并随浏览器持久化往返；submitTurn 按当前会话 overview→冻结值→激活值→default 链取值），消除"会话在 B 组、工具在 A root 执行"错位；sessions.ts 两个 overview builder 透传归属（分组漂移修复）。
  - **P2 批次**：loadWorkspaces 失败不锁死且照常归一（防孤儿 id 首轮盖章）；新建失败可见反馈；空态 v-else 防双渲染；行 key 稳定复合键；持久化双轨收敛至 sidebar-groups 单源（原始读+裁剪写）；组头快捷新建按钮（design §2）；分组函数去无用参数。
- **验证（终态）**：vitest **414 passed + 10 skipped（23 文件）全绿**；vue-tsc + vite build 通过；openspec validate --strict 通过；已归档并提交（fd0353d）。

## Next Action

- 无（已收口）。后续候选：会话跨 workspace 移动、workspace 内文件浏览器（Non-Goals 登记项）。

## Blockers

- 依赖 PA-079（数据模型）与 PA-080（权限反馈语义）

## Resume Hint

- 先读 `src/components/HomeSessionSidebar.vue`、`src/stores/runtime.ts` 的 sessionList/sessionId、`src/lib/runtime/sessions.ts`
