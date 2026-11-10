# ADR 0116：会话时间线键盘导航（Ctrl+T/Ctrl+B 顶部/底部 + Alt+↑/↓ 按轮跳转）

Status: implemented

日期：2026-11-10
实施说明：[`.agents/notes/implemented/feature/2026-11-10-workbench-timeline-keyboard-navigation.md`](../../.agents/notes/implemented/feature/2026-11-10-workbench-timeline-keyboard-navigation.md)。
关联：[0113](0113-tui-esc-double-press-rewind.md)（TUI 双击 Esc 撤回，同为「会话内快速定位」交互，键位各自裁决）/ [0058](0058-pds-budget-reducer-adoption.md)（时间线自动跟随的视口阈值与暂停/恢复语义，本卡复用不重定义）。

## 背景

长会话里仅靠鼠标滚轮回到顶部或定位到某一轮成本很高。工作区已有按轮跳转能力（`TurnDial`
点击刻度 → `handleSelectDialTurn`），但缺少键盘可达性；也没有「一键到顶 / 一键到底」的快捷入口
（到底只能点浮动的「滚动到最新」按钮）。

用户诉求：为会话时间线提供键盘快捷键——快速回顶部、回底部，以及在相邻轮次之间前后跳转。

## 候选方案

- **A（采纳）Ctrl/Cmd+T 顶部、Ctrl/Cmd+B 底部、Alt+↑ 上一轮、Alt+↓ 下一轮**：
  - 底部复用既有收口 `handleScrollToBottom`（清未读 + 恢复自动跟随，单一真源，避免第二套到底逻辑）；
  - 顶部与按轮跳转复用拨盘已有的 `viewport.scrollTo` + `activeTurnIdForDial` 语义，滚动事件自然被视为
    「用户滚动意图」→ 暂停自动跟随，与点击拨盘刻度同口径，不新增暂停/恢复 API。
- **B（落选）Ctrl+Home / Ctrl+End 顶部 / 底部**：初稿曾采用，评审时用户明确要求改为 Ctrl+T / Ctrl+B
  （更顺手、与 WebView/系统窗格的 Home/End 语义解耦）。`Ctrl+Home/Ctrl+End` 不再被本功能拦截。
- **C（落选）只做鼠标拖动滚动条**：`ScrollArea` 以 `hide-scrollbar` 隐藏原生滚动条以保流式居中对齐，
  恢复可见滚动条会改变视觉与对齐，且不解决键盘可达性。
- **D（落选）Alt+↑/↓ 改用裸 ArrowUp/ArrowDown**：裸 ArrowUp 已被输入框「回填最近用户消息」占用，
  裸方向键还有多行光标移动语义；加 Alt 修饰可与之互斥、零冲突。

## 决策

1. `src/components/HomeWorkspace.vue` 的窗口 `keydown` 统一入口（`handleWindowKeydown`）新增分支，
   并跳过 IME 组合态（`isComposing` / `keyCode 229`）：
   - `Ctrl/Cmd+T` → 视口 `scrollTo({ top: 0, behavior: "smooth" })`；
   - `Ctrl/Cmd+B` → `handleScrollToBottom()`（复用「滚动到最新」收口）；
   - `Alt+↑` / `Alt+↓` → 在当前可见轮次的相邻轮之间跳转（首尾夹取、不越界），滚动后同步 `activeTurnIdForDial`。
2. 轮次节点的收集收口为 `getTimelineTurnElements()`（优先 `workspaceContentColumnRef`，退化到滚动视口），
   `handleSelectDialTurn` 改为同源复用；不再各写一份 `querySelector`。
3. `src/components/chat/TurnDial.vue` 的「激活轮居中」watch 补 `typeof scroller.scrollTo === "function"` 守卫：
   键盘/按钮切换激活轮会触发该 watch，非浏览器环境（jsdom/单测）元素无 `scrollTo`，缺守卫会抛未捕获 rejection。
4. 新增本地取证 debug 事件 `keyboard-nav:scroll-to-top` / `keyboard-nav:scroll-to-bottom` / `keyboard-nav:turn`，
   仅写前端诊断缓冲，不新增遥测出口。四维归属维持结构性 N-A（无新增网络出口 / 指标 / 上报路径 /
   enforcement_point），故本次**无需修改** `docs/observability.yaml`。

## 后果

- 纯前端交互变更：无新增网络出口、无持久化 schema 变更、无 LLM 调用；可观测性 tracing/metrics/logs/issues
  维持结构性 N-A（声明文件不需要改动）。
- 键位占用：`Ctrl+T` / `Ctrl+B` 在桌面 WebView 无默认冲突（应用内无浏览器标签页）；`Alt+↑/↓` 与输入框
  既有「裸 ArrowUp 回填历史」路径互斥（后者仅在无修饰键时生效）。
- 自动化回归：`tests/HomeWorkspace.spec.ts` 新增 4 条用例，覆盖「顶部」「底部」「上/下一轮」「旧键位
  Ctrl+Home 不再触发自定义滚动」。
