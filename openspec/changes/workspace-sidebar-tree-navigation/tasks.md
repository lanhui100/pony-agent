# Tasks

- [ ] `runtime.ts`：`workspaceList` / `activeWorkspaceId` 状态（初值读 `pony-agent.active-workspace.v1`，不在列表则回退 `"default"` 并清理）+ `createWorkspace`（调 `workspace_create`，成功自动激活）+ `activateWorkspace`（仅写 localStorage）+ `createSession`/`submitTurn` 携带 `workspaceId: activeWorkspaceId ?? "default"`。
- [ ] `src/lib/runtime/workspace-constants.ts`：`DEFAULT_WORKSPACE_ID = "default"` 单源导出（与 PA-079 后端常量对齐，跨端测试断言）。
- [ ] `HomeSessionSidebar.vue`：分组 computed（`groupedSessions`）、组头渲染（名称/计数/折叠）、会话项渲染复用、折叠状态 localStorage 持久化（`pony-agent.session-sidebar-workspace-groups.v1`，读取忽略未知 key）、激活组仅无持久化值时默认展开。
- [ ] 瞬态"新对话"条目携带 `workspaceId: activeWorkspaceId ?? "default"`；孤儿 `workspaceId` → "未分组"组（会话仍可切换/删除）。
- [ ] "显示全部"解除所有组会话数上限（组计数按全量计算）；不做 per-group 分页。
- [ ] Workspace 管理入口：列表 + 新建（名称/路径）+ 激活切换；浏览器模式禁用。
- [ ] 空态："暂无对话"；浏览器模式仅默认组。
- [ ] 前端单测：分组正确性（含 None→default、瞬态条目归激活组）、折叠持久化（write→remount-read）、激活切换（不隐藏其他组）、空态、新建 workspace 自动激活、孤儿组、浏览器模式、过期折叠 key 忽略。
- [ ] `npm run test:unit`、`npm run build` 通过；既有 sidebar 测试更新后全绿。

## Validation Notes

- 3 路对抗审核（2026-08-09）已采纳：激活 localStorage 单一真相源（T-7/C-4）、AC2 语义改写"切换不隐藏组"（T-6）、瞬态条目带 workspaceId（C-12/R-21）、孤儿"未分组"（C-14）、分页×分组"显示全部"（C-13/R-22）、"default" 共享常量 + 跨端断言（T-16/C-5）、浏览器模式与过期状态场景（T-18）、localStorage key 前缀对齐（R-23）。
