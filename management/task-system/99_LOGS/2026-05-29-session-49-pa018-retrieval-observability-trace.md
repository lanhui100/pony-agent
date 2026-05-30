# 2026-05-29 Session 49 - PA-018 Retrieval UI 归位到右侧 Observability / Trace

## 背景
- 用户确认了前端产品边界：
  - 左侧边栏承载菜单与对话历史，不承载 retrieval 常驻信息。
  - 中间对话主区域承载消息流，不承载 retrieval 顶部条。
  - 右侧边栏承载 observability，包含 `status / trace / tools / attachments`，retrieval 应整合到该区域。
- 既有实现把 retrieval 分散放到了：
  - 左栏 `current context`
  - 中栏 `Retrieved Context / Run State`
  - 右栏 `status` 面板中的 `Retrieval`

## 本轮实现
- 移除了左侧会话边栏中的 retrieval 当前上下文卡片。
- 保持中间工作区顶部 retrieval strip 与 run/checkpoint strip 不再显示。
- 将 retrieval 摘要统一整合进右侧 `trace` 面板顶部，保留：
  - retrieval summary
  - run goal
  - active task
  - last referenced file
  - memory preview
- `status` 面板仅保留本次对话整体状态与运行元信息，不再承载 retrieval 卡片。

## 代码结果
- 重建并清理以下前端文件的实现与测试，使其回到可维护 UTF-8 状态：
  - `src/components/HomeSessionSidebar.vue`
  - `src/components/HomeSidebar.vue`
  - `tests/HomeSessionSidebar.spec.ts`
  - `tests/HomeSidebar.spec.ts`
  - `tests/HomeWorkspace.spec.ts`
- 现有前端边界文档继续作为后续开发约束：
  - `docs/guides/frontend-layout-and-observability-boundary.md`

## 验证
- `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts`
- `npm exec vue-tsc -- --noEmit`

## 状态
- `PA-018`：保持 `In Progress`
- 本轮完成的是 retrieval UI 布局归位与可观测性边界收口，不宣告任务完结。
