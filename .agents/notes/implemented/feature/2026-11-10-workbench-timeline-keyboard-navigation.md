# Agent Note: 会话时间线键盘导航（ADR 0116 实施记录）

Status: implemented

- 日期：2026-11-10
- 决策真源：[`docs/decisions/0116-workbench-timeline-keyboard-navigation.md`](../../../../docs/decisions/0116-workbench-timeline-keyboard-navigation.md)
- 交付面：桌面工作区前端（Vue 3 / TypeScript）

## Problem

长会话里回到顶部、回到底部或按轮定位只能靠鼠标：到底要浮动「滚动到最新」按钮，按轮要点击 `TurnDial`
刻度，缺少键盘可达性。用户在评审中要求补一组快捷键：顶部 / 底部 / 上一轮 / 下一轮，并把顶部 / 底部
的键位从初稿的 `Ctrl+Home` / `Ctrl+End` 改为 `Ctrl+T` / `Ctrl+B`。

## Decision

改动集中在三处，均为前端交互层：

1. `src/components/HomeWorkspace.vue`
   - `handleWindowKeydown` 新增分支（IME 组合态跳过）：`Ctrl/Cmd+T` → 顶部；`Ctrl/Cmd+B` → 复用
     `handleScrollToBottom()`；`Alt+↑` / `Alt+↓` → 相邻轮跳转（首尾夹取）。
   - 新增 `getTimelineTurnElements()`（优先 `workspaceContentColumnRef`，退化到滚动视口）收口轮次节点收集；
     `handleSelectDialTurn` 改为同源复用，避免第二份选择器。
   - 新增 `scrollToTurnElement` / `handleNavigateTurn` / `handleTimelineScrollToTop` /
     `handleTimelineScrollToBottom`，以及本地 debug 事件 `keyboard-nav:*`。
2. `src/components/chat/TurnDial.vue`
   - 「激活轮居中」watch 补 `typeof scroller.scrollTo === "function"` 守卫。键盘/按钮切换激活轮会触发该
     watch；jsdom 下元素无 `scrollTo`，缺守卫会抛未捕获 rejection（单测会以 Errors 计并可能拉红 `vitest run`）。
3. `tests/HomeWorkspace.spec.ts`
   - 新增 `describe("keyboard timeline navigation")` 四条用例：顶部（校验 `scrollTo({top:0})` + debug 事件）、
     底部（校验复用到 `resume-auto-follow`）、Alt+↑/↓（按 mock `offsetTop` 校验落点与 debug 事件方向）、
     旧键位 `Ctrl+Home` 不触发自定义滚动。

## Alternatives considered

- **Ctrl+Home / Ctrl+End**：初稿采用；用户评审改为 `Ctrl+T`/`Ctrl+B`（更顺手，且与 WebView/系统窗格的
  Home/End 语义解耦）。已加负面用例锁定「Ctrl+Home 不再触发自定义滚动」。
- **只做鼠标拖动滚动条**：`ScrollArea` 以 `hide-scrollbar` 隐藏原生滚动条以保流式对齐，恢复可见滚动条会
  改变视觉与对齐，且不解决键盘可达性。
- **裸 ArrowUp/ArrowDown 做轮跳转**：与输入框「裸 ArrowUp 回填最近用户消息」及多行光标移动冲突；
  改用 `Alt+↑/↓` 互斥、零冲突。
- **到底另写一套 scrollToBottom**：不采用。复用 `handleScrollToBottom` 保留「清未读 + 恢复自动跟随」单一真源。

## Consequences

- 纯前端交互：无新增网络出口、持久化结构、LLM 调用；可观测性四维维持结构性 N-A，本次无需修改
  `docs/observability.yaml`（无新增遥测面 / enforcement_point）。
- 顶部 / 按轮跳转不新增暂停自动跟随 API：`scrollTo` 触发的滚动事件按既有「用户滚动意图」路径暂停跟随，
  与点击拨盘刻度同口径。
- 机器回执：`npx vitest run tests/HomeWorkspace.spec.ts` 全绿（77 passed / 7 skipped，含新增 4 条）；
  `npx vue-tsc --noEmit` 通过；`node scripts/check-decisions.mjs` 通过（0116 已入索引）。
