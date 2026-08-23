# 前端开发指南

## 定位

前端不是单纯聊天页面，而是 Pony Agent 的“运行时调试台”。

## 推荐技术选择

- Vue 3
- TypeScript
- Pinia
- Vite
- Tailwind CSS
- shadcn-vue 风格组件

## 为什么现在建议上 Pinia

因为很快会出现这些全局状态：

- 当前会话
- 当前消息流
- 当前运行阶段
- 工具调用列表
- Provider 状态

如果没有集中状态管理，后续组件一多会很乱。

## 路由策略

当前通过 `App.vue` 的 `currentPage` 条件渲染实现 3 个页面的切换（home / config / telemetry），未引入 vue-router。配置页内部以受控 tab（通用/模型/工具）承载原独立设置面，遥测页以 tab（Trace/指标）承载 trace 与 metrics 读面。

如果后续页面继续增长（workflow 设计器、附件中心独立页等），应重新评估引入 vue-router 的必要性。

## 页面建议

### 对话工作区（HomeWorkspace.vue）

- 消息流展示（按 TurnBucket 分组：user/assistant/tools）
- 用户输入与提交
- 流式消息实时展示
- checkpoint/history 控制

### 左侧导航栏（HomeSessionSidebar.vue）

- 品牌入口、新对话
- 工作区（第一优先：管理/新建/切换，头部显示激活工作区名）
- 对话（按 workspace 分组的会话树）
- 底部一级导航：遥测/指标、模型配置、设置

### 右侧对话过程面板（HomeSidebar.vue）

只承载对话过程信息（PA-096 起）：

- Status：会话轮次/token 聚合、上下文用量、运行状态摘要
- Plan：计划列表与步骤控制
- Diagnostics：诊断信息

Trace 与 metrics 读面在遥测页（`telemetry/TelemetryPage.vue`），工具目录在配置页工具 tab。

## 当前 UI 风格约束

- 极简
- 中文优先
- 微圆角
- 布局紧凑
- 优先解释运行时，而不是装饰页面

## 为什么引入 shadcn-vue + Tailwind

- 组件风格统一
- 调整密度和圆角很方便
- 更适合快速迭代学习型工作台
- 比完全手写 CSS 更容易形成可维护的设计系统

## 前端最佳实践

- 用组件边界映射领域边界
- 用 store 管理领域状态
- 用 UI 可视化帮助理解智能体行为
