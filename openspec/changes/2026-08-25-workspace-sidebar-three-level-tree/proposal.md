# 侧边栏工作区三级树：单一树 IA、会话级操作与归属语义收敛

## 为什么现在

侧边栏现状是「工作区」折叠管理区块（列表行 + 内联新建表单）与「对话」区块按工作区分组两套并存的 IA：工作区管理与会话浏览割裂、组可折叠但需求要求工作区恒展开为一级菜单、新建工作区要手填 root 路径。产品需求明确为单一树（工作区一级标题行不可折叠 → 各工作区二级 → 其下对话三级），并补齐会话级操作（重命名/归档/删除入三点菜单）。后端能力核查显示：工作区重命名/删除、会话重命名/归档四项命令完全缺失；会话标题虽按首条用户消息自动派生，但每轮持久化覆盖，无法支撑重命名；归档概念不存在。同时发现删除工作区若只移除注册，名下会话的 `workspace_id` 将在附件导入（硬失败）与工具执行（静默回退默认 root）两条运行时路径上行为分叉，需要一并裁决归属语义。

## 变更范围

- 后端（crates/pony-agent-core、src-tauri）：
  - `SessionState` 新增 `titleOverride`（Option，serde default）与 `archived`（bool，serde default）两个持久化字段；统一 `effective_title()` 投影点位（list_sessions / snapshot / refresh_session_metadata 全部分支）；`SessionOverview` 投影 `archived`。
  - `workspace.rs` 新增 `rename_workspace_entry`（trim 非空、≤64 字符、拒绝未知 id 与重名、id/root 不变）、`delete_workspace_entry`（拒绝 `default` 与未知 id；删除时把名下会话 `workspaceId` 批量重写为 `default` 并落盘）。
  - 新增四个 Tauri 命令：`workspace_rename` / `workspace_delete` / `session_rename` / `session_archive`。
  - 引入 tauri-plugin-dialog（最小权限 `dialog:allow-open`），支撑「添加工作区」的目录选择。
- 前端（src/lib/runtime、src/stores/runtime.ts、src/components/HomeSessionSidebar.vue、src/components/ui）：
  - `sidebar-groups.ts` 重写为 `deriveSidebarTree()`：平铺区（默认 + 孤儿，updatedAtMs 降序、conversationId 升序，瞬态钉顶）+ 工作区组（含零计数组）。
  - HomeSessionSidebar 单一树重建；新增 DropdownMenu 基件；ConfirmPopover 升级为受控异步确认（open/loading/错误槽）。
  - store 四个新 action + inflight 守卫；顶层「新对话」显式落默认工作区；废弃组折叠持久化。

## 非目标

- 不做归档恢复入口（unarchive 命令与归档列表 UI 留待后续迭代；字段设计为其预留）。
- 不改变工作区注册表的存储位置（store_metadata key=`workspaces`）与默认工作区初始化机制。
- 不改后端 `build_title` 派生算法本身（仍以首条用户消息命名新会话）。
- 不引入「最近激活工作区栈」，`activeWorkspaceId` 保持单一真相源（本方案后顶层按钮不再依赖隐式激活）。
- 不处理多窗口场景（现仅 main 窗口）。

## 验收标准（摘要，细则见 tasks.md）

1. 单一树渲染：一级标题行无折叠交互且右端出 FolderPlus 图标按钮（tooltip「添加工作区」）；二级行含文件夹图标/名称/计数徽标/悬停纯图标＋与三点菜单；三级行为截断标题 + 三点菜单（重命名/归档/删除）。
2. 平铺区排序契约：默认 ∪ 孤儿按 `updatedAtMs desc`、tie `conversationId asc`，瞬态钉顶；组内同规则；零计数组保留空态。
3. 四个新命令全链路可用且过 cargo 测试矩阵（校验/幂等/往返/default 拒删/归属重写后 resolve 成功）。
4. 会话重命名后连发一轮（history 分支与 trace 分支）标题均保持 override；checkout 回灌不丢。
5. 归档二次确认文案含「当前版本无法从界面恢复」；确认后行消失且重启仍隐藏。
6. 删除工作区二次确认文案含三要素 + 工具根披露句；删除激活工作区后激活态回退 default。
7. 浏览器降级矩阵逐格成立（管理入口隐藏；会话菜单仅保留本地可用的删除）。
8. 门禁：vitest / vue-tsc / cargo check / cargo:test:lib 通过；双 reviewer 对抗审核每阶段闭环。
