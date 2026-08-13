# Design

## Decision Summary

1. `HomeSessionSidebar.vue` 平铺列表改为分组树：外层 v-for workspace 组（可折叠），内层 v-for 会话。
2. `runtime.ts` 增加 `workspaceList` / `activeWorkspaceId`；`createWorkspace` 调 PA-079 host command（`workspace_create`/`workspace_list`）；`activateWorkspace` 仅写 localStorage（单一真相源，PA-079 无后端激活）。
3. 分组用 computed 一次成型：`groupedSessions = Map<workspaceId, SessionOverview[]>`，渲染不再逐次 filter。
4. 折叠状态存 localStorage（对齐既有 `pony-agent.session-sidebar-*` 前缀：`pony-agent.session-sidebar-workspace-groups.v1`），激活组默认展开（仅当无持久化值）。
5. 瞬态"新对话"条目携带 `workspaceId = activeWorkspaceId ?? "default"`，归入正确组。
6. 孤儿 `workspaceId`（注册表无记录）→ 合成"未分组"组，不崩溃。

## Chosen Direction

### 1. store 状态（runtime.ts）

```ts
workspaceList: WorkspaceOverview[];        // { id, name, rootPath }
activeWorkspaceId: string | null;
async createWorkspace(name, rootPath): Promise<WorkspaceOverview | null>;  // 调 workspace_create，成功后自动激活（写 localStorage）
async activateWorkspace(id): Promise<void>;  // 仅写 localStorage `pony-agent.active-workspace.v1`
```

- `init` 时拉取 `workspace_list`（Tauri）；不可用时（浏览器）合成默认组 `{ id: "default", name: "默认工作区", rootPath: null }`。
- `activeWorkspaceId` 初值从 `pony-agent.active-workspace.v1` 读取；该值不在 `workspaceList` 时回退 `"default"` 并清理存储。
- `createSession()` / `submitTurn` 携带 `workspaceId: activeWorkspaceId ?? "default"`（PA-079 经 `TurnInput.workspaceId` 落地）。
- `"default"` 常量与 PA-079 共享单源导出（`src/lib/runtime/workspace-constants.ts`），跨端测试断言一致。

### 2. 侧边栏分组（HomeSessionSidebar.vue）

```ts
const groupedSessions = computed(() => {
  const groups = new Map<string, SessionOverview[]>();
  for (const session of visibleSessions.value) {
    const key = session.workspaceId ?? "default";
    (groups.get(key) ?? groups.set(key, []).get(key)!).push(session);
  }
  return [...groups.entries()].map(([id, sessions]) => ({
    workspace: workspaceById(id) ?? unknownWorkspace(id),  // 孤儿 → "未分组"
    sessions,
  }));
});
```

- 组头：workspace 名 + 会话数 + 折叠箭头（`ChevronDown/ChevronRight`）+ "新建对话"快捷按钮。
- 会话项渲染复用既有逻辑（headline/时间/删除/切换）。
- 折叠状态：`Map<workspaceId, boolean>` ref + localStorage 同步（key=`pony-agent.session-sidebar-workspace-groups.v1`）；读取时忽略未知 key（防御清理）；`activeWorkspaceId` 组仅当无持久化值时默认展开；**当前会话所属组自动展开仅当该组本 boot 内无用户折叠记录**（用户显式折叠优先，消除"自动展开 vs 持久化折叠"冲突）。
- 瞬态条目：合成当前会话 overview 时写 `workspaceId: activeWorkspaceId ?? "default"`，确保"新建对话"落在激活组。
- 孤儿组：`workspaceById(id)` 返回 undefined 时，组头显示"未分组"，会话仍可切换/删除（不丢数据）。

### 3. 分页 × 分组

- 现有 `visibleConversationCount`"显示更多"改为组级视角："显示全部"移除**所有组**的会话数上限（一次性展示全部组内会话）；不做 per-group 分页（非目标）。
- 组计数始终按全量会话（含未显示部分）展示。

### 4. Workspace 管理入口

- 侧边栏底部（或顶部品牌区下方）"工作区管理"按钮 → 小面板：列表 + 新建（名称/路径输入）+ 激活切换。
- 新建成功后自动激活（写 localStorage）；激活即持久化。
- 浏览器模式：管理入口隐藏或禁用（避免 host command 失败噪音）。

### 5. 空态与降级

- 组内无会话 → "暂无对话" 提示。
- 非 Tauri（浏览器预览）→ 只显示默认组，管理入口禁用。
- 会话 `workspaceId` 无注册表记录 → "未分组"组（见 §2）。
- localStorage 折叠状态含未知 workspaceId key → 忽略，不崩溃。

## Edge Cases

- 当前会话不属于激活 workspace：仍归其归属组并高亮；激活组不变。
- 删除会话后分组计数即时更新（computed 自动）。
- 折叠状态中删除 workspace 记录（PA-079 无删除场景时忽略，仅防御清理）。
- 激活 workspace 与折叠冲突：以持久化折叠值为准；激活时仅当无持久化值才默认展开该组。
- 浏览器模式 + 持久化残留：`activeWorkspaceId` 不在列表 → 回退 `"default"` 并清理。
- 权限提示（读 workspace 外文件被拒）消费 PA-080 共享错误信封 `{ code, message }`（如 `requires_authorization`）；PA-081 只渲染提示，不定义错误结构。
