# Agent Note: 前端分层倒置清理、组件越界收口与设计系统单源收敛

Status: proposed

## Problem

前端代码库（`src/` 160 文件 / 48,540 行）虽已开展 `stores/runtime/` 拆分与流式事件收口（参见 [2026-11-22-stream-sink-nonblocking-emit-contract.md](../../implemented/architecture/2026-11-22-stream-sink-nonblocking-emit-contract.md)），但仍存在严重的分层倒置、环依赖、状态越界写入与设计系统分裂问题：

1. **生产构建反向依赖 AI Skill 目录**：`src/styles.css:5-6` 直接 `@import "../.agents/skills/streaming-motion/tokens.css"` 与 `primitives.css`，将开发期治理技能目录变成了前端生产构建的硬性静态依赖。
2. **`lib ↔ stores` 环依赖与无效动态导入**：
   - `src/lib/tauri.ts:99` 动态 `await import("@/stores/runtime")` 读取 `activeTraceId`，而全部 store 依赖 `@/lib/tauri`；
   - `src/lib/runtime/useTraceProjection.ts:4-5` 处于 `src/lib/` 工具层却静态导入 `@/stores/providers` 与 `@/stores/runtime`；
   - `src/stores/runtime/orchestration.ts` 动态导入 `@/stores/ask`；`vite build` 已明确报出 2 条 `INEFFECTIVE_DYNAMIC_IMPORT`。
3. **组件绕过 Store 直调 IPC 与越界写状态**：
   - `GoalBar.vue:80-156` 绕过 `useGoalStore` 自建局部状态、直调 `goal_budget_lines` 并直连 4 条 `goal://*` 监听；
   - `ModelMonitorPage.vue:191,223,258,290,311` 重复封装 `stores/runtime/catalog.ts` 已提供的 3 条能力查询 IPC；
   - `HomeWorkspace.vue:1555,1592,1601,1612` 直接写 `runtimeStore.draftMessage` / `workspacePanelsStore.activePanel` 并直调 `goal_command`/`compact_session`；
   - `ProviderConfigPage.vue:708,997,1076,1097` 直接写 `providerStore.notice`；`useSettingsStore` 存在顶层字段与 `settings.*` 双源字段。
4. **设计系统单源性缺失与重复 UI 基件**：
   - `components.json:5-9` 声明 `shadcn-vue` + CSS 变量，但 `src/styles.css` 无对应语义 Token，全仓散落 **102 处硬编码 hex 色值**；
   - `InfoTip.vue`（手写 `<Teleport>` + 全局 `window` `resize`/`scroll` 监听，深色主题）与 `ui/Tooltip.vue`（Reka-UI，浅色主题 + 依赖未安装 `tailwindcss-animate` 的死类名）并存，且 `<TooltipProvider>` 冗余挂载 4 处；
   - `HomeTracePanel.vue` 与 `ModelMonitorPage.vue` 拷贝 7 个格式化/标签函数；`ConfigPage.vue` 与 `TelemetryPage.vue` 拷贝 Tablist 键盘导航；`motion-v` 全局注册仅服务 4 处 `v-motion`。

## Proposal

建议采用渐进式架构收敛方案（分两阶段实施，不改变用户可见视觉交互）：

1. **切断 `.agents/` 生产依赖与 `lib ↔ stores` 环依赖**：
   - 将流式动效 Token/原语迁入前端自有资产目录 `src/styles/motion/`（由 `.agents/skills/streaming-motion` 通过文档或同步脚本单向校验），`src/styles.css` 仅导入 `src/` 内部样式；
   - 重构 `src/lib/tauri.ts`，通过启动期注册回调 `configureTauriRuntimeContext({ getActiveTraceId })`（在 `src/main.ts` 注入）取代反向导入 `@/stores/runtime`；
   - 将 `useTraceProjection.ts` 从 `src/lib/runtime/` 迁入 `src/composables/`（明确位于 `stores/` 之上、组件之下），同步把散落在 `src/lib/use*.ts` 的 composable 归位。
2. **收口 Store 唯一写者与移除组件直调 IPC**：
   - 将 `GoalBar.vue` 的 `budgetLines` 与 `goal://*` 订阅收归 `useGoalStore`；将 `ModelMonitorPage.vue` 能力查询收敛到 `stores/runtime/catalog.ts`；将 `HomeWorkspace.vue` 与 `ProviderConfigPage.vue` 对 store 属性的直接赋值改为显式 action。
3. **设计系统单源化与重复 UI 清理**：
   - 在 `src/styles.css` 通过 Tailwind v4 `@theme` 收敛高频语义色与动效变量（消减 102 处 hex 硬编码），并修复或移除 `ui/Tooltip.vue` 无效的 `animate-in`/`fade-in-0` 类名；
   - 将 `InfoTip.vue` 统一改为基于 `ui/Tooltip.vue` 的薄变体（移除无条件注册的 `window` 事件监听器），全仓收口至根 `App.vue` 唯一 `<TooltipProvider>`；
   - 将 `HomeTracePanel.vue` 与 `ModelMonitorPage.vue` 共享的 7 个格式化/标签函数抽入 `src/lib/runtime/trace-labels.ts`，将 Tablist 键盘导航抽为 `useTablistNavigation` composable，评估移除仅 4 处使用的 `motion-v` 改用 CSS/Web Animations。

## Alternatives considered

- **方案 A：资产下沉 `src/styles/motion/` + 回调注入去环 + Tailwind `@theme` 渐进收敛（推荐）**。改动局部可控，立刻消除生产对 `.agents/` 的逆向依赖、消除 `vite build` 动态导入告警及 Tooltip 全局监听泄漏，且可按模块分 PR 验收。
- **方案 B：仅在文档层面说明保留现状，不改代码分层**。落选：只要 `styles.css` 继续依赖 `.agents/skills/` 且 `GoalBar`/`ModelMonitorPage` 继续绕过 Store，前端就无法建立真实状态单源，且任何删除/重构 skill 目录的操作都会打断前端构建。
- **方案 C：一次性引入完整 `shadcn-vue` 组件库重写全部 7 个 >1000L 页面**。落选：爆炸半径覆盖 48.5k 行 UI 代码，难以在不破坏现有暖色设计（`stone`/`amber`）与 E2E 测试的前提下一次交付。

## Acceptance criteria

1. `rg "\.agents/" src/` 返回 0 命中；`npm run build` 不再出现 `INEFFECTIVE_DYNAMIC_IMPORT` 告警。
2. `src/lib/` 目录下零 `@/stores/` 导入（`rg "@/stores" src/lib/` 返回 0 命中）。
3. 业务组件中（除 `WindowControls.vue`、`workspace-api.ts`、`tauri-updater.ts` 系统原语外）零绕过 Store 的重复 IPC 调用；`<TooltipProvider>` 仅在 `App.vue` 挂载 1 次，`InfoTip.vue` 不再向 `window` 注册全局监听器。
4. `npm run typecheck`、`npm run test:unit` 与现有 Playwright E2E / UI 守护测试全绿。

## Risks

- 修改 `useGoalStore` / `GoalBar.vue` 事件订阅时序可能影响 Goal 进度条实时刷新，需结合 `tests/goal-store.spec.ts` 与相关 E2E 验证。
- 合并 `InfoTip.vue` 与 `ui/Tooltip.vue` 时需保留深色/浅色变体 props 与无障碍触发属性，避免现有组件测试及 UI 视觉回归。
