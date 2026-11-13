# Agent Note: Goal 面板快捷键 (Ctrl+G) 与快捷键帮助弹窗 (Ctrl+H)

Status: implemented

- 日期：2026-11-12
- 状态：implemented
- 作用域：前端桌面（Vue 3 / TypeScript）

## Context

Pony Agent 桌面端已具备目标（Goal）、计划与任务（Plan/Todos）、团队看板等丰富功能。
在前期波次中，已在 Rust core 实现了 `/goal` 斜杠命令的语法与状态机真源。
然而此前：
1. 右侧栏共享 store `workspace-panels` 仅声明了 `"plan" | "roster" | "tasks"` 三个面板，GoalBar 作为一个常驻组件无法通过快捷键折叠/展开；
2. 缺乏全局快捷键直达 Goal 面板的入口；
3. 系统拥有多套快捷键（Ctrl+Z 撤销、双击 Esc 停止、Alt+S 切换排队/插队、Ctrl+T/B 顶部底部滚动、Alt+↑/↓ 轮次跳转等），但用户无法随时查阅系统快捷键清单。

## Decision

1. **扩展右侧工作区面板类型**：
   - 将 `WorkspacePanel` 联合类型扩充为 `"plan" | "roster" | "tasks" | "goal"`；
   - 在 `workspace-panels.ts` 中暴露 `toggleGoal()` 方法，保持与其余面板一致的互斥与开合语义；
2. **重构 GoalBar 兼容受控折叠与空态引导**：
   - 支持受控 `open` 属性：未传入时保持旧契约（有目标即显，无目标不渲染 `goal-bar`）；显式传入时支持无目标情况下的优雅空态引导（提示通过 `/goal <目标>` 创建）；
   - 带有统一的折叠头（`Target` 图标、轮次指示器、状态 Badge、`ChevronRight` 展开角标）；
3. **快捷键 Ctrl+G（Mac: Cmd+G）直达 Goal 面板**：
   - 在 `HomeWorkspace.vue` 的全局键盘监听中绑定 `Ctrl/Cmd+G`；
   - 触发时切换 Goal 面板开合；在无目标时展开并提示创建引导；
   - `/goal`（无参数）命令命中后，自动切换并展开 Goal 面板；
4. **新增 Ctrl+H（Mac: Cmd+H）快捷键帮助弹窗**：
   - 新建 `ShortcutsHelpModal.vue`（基于 `reka-ui` 的 DialogPrimitive 与 Portal 传送），展示结构化快捷键清单（目标与计划、对话与发送、会话与导航、快捷命令与通用）；
   - 在全局监听中拦截 `Ctrl/Cmd+H`（阻止浏览器默认帮助或历史记录快捷键），实现一键呼出与 Esc 关闭。

## Alternatives considered

1. **以 Drawer 侧边抽屉替代 Dialog 弹窗展示快捷键**：
   - *未采纳*：快捷键是即时查询性质的高频参考工具，模态居中 Dialog 更契合用户视线中心，不挤占会话工作区。
2. **把 Goal 做成独立模态弹窗而非侧边栏面板**：
   - *未采纳*：Goal 具有长运行持续感知属性，与 Plan、Tasks 同属侧边面板上下文；将其纳入 `WorkspacePanel` 能保持右侧栏状态机的一致性与低耦合。

## Machine Evidence

- `tests/goal-panel-and-shortcuts.spec.ts`（4 passed）
- `tests/workspace-goal-shortcuts.spec.ts`（4 passed）
- `tests/GoalBar.spec.ts`（3 passed，冻结回归通过）
- `npm run typecheck`（exit 0）
