# Proposal: workspace-nav-observation-entry-and-provider-hierarchy

## Why

ADR 0009 确立的导航运行后反馈：左栏一级键"模型配置"与配置页"模型"tab 双入口冗余；"遥测"数据面向特定对话回看而非常驻导航心智，且更名"观测"后入口应贴近其会话上下文；ProviderConfigPage 的"提供商手风琴 + 右侧单编辑器"在实体增多后层次不清，编辑对象歧义。

## What Changes

信息架构调整（纯前端，`src/` 层）：

1. **左栏一级键精简**：删除"模型配置"与"遥测|指标"两个一级键（含折叠态图标）；左栏底部一级导航仅剩"设置"；`SidebarNavigationPage` 收窄为 `"home" | "settings"`；config·models/tools tab 派生高亮为空。
2. **观测入口右置**：对话页右栏右上角折叠按钮左侧新增纯图标浮动按钮（Activity 图标，tooltip"观测"，aria-label"观测"）；点击进入二级观测页（原遥测页）；任何窗口宽度常显——`<1000px` 折叠按钮隐藏时观测按钮落位其原位（right-3），不重演 0009 否决的窄窗口零入口。
3. **更名**：页头 coding/work 统一为"观测"；tablist aria-label 改"观测视图切换"；工作模式卡片描述同步改写；ModelMonitorPage 内嵌头"模型监控"→"指标监控"。双 tab 门禁不变。
4. **提供商页层次折叠**：ProviderConfigPage 左列扁平化（去手风琴，点击即选中）；右侧恰两个一级可折叠 section——提供商详情（view/edit/create 三态+专属动作条）、模型列表（行内展开"模型配置详情"、新增模型表单卡、行级动作条）；create-provider 时模型列表 section 隐藏；表单编辑中锁定所在区折叠；页头只承载当前提供商名。
5. **决策记录**：新增 ADR 0013（承接重述 0009 仍有效决定），0009 移入 superseded/。

## Scope

- 修改：`src/types/config.ts`、`src/App.vue`、`src/components/HomeSessionSidebar.vue`、`src/components/telemetry/TelemetryPage.vue`、`src/components/config/ConfigGeneralSection.vue`、`src/components/ModelMonitorPage.vue`、`src/components/ProviderConfigPage.vue`
- 测试：改写 `tests/App.spec.ts`、`tests/HomeSessionSidebar.spec.ts`、`tests/TelemetryPage.spec.ts`；新增 `tests/ProviderConfigPage.spec.ts`
- 文档：`docs/decisions/{README.md,0013-*.md,superseded/0009-*.md}`、`docs/guides/frontend-layout-and-observability-boundary.md`

## Non-Goals

- 不改 runtime/providers 等 Pinia store 数据流、Rust 后端与 Tauri command
- 不引入 vue-router
- 不将 metrics 改为按当前会话过滤（保持全局聚合 + 按会话下钻，口径见 ADR 0013）
- 不为脏表单添加离开确认门槛（记录为后续项）
- vitest coverage.include 清单与 `test:ui-guard` 文件集不变

## Task Tracking

本变更随开发会话执行（dev-team A 级路径，plan/spec 经双独立对抗审核）；任务卡不入 `management/task-system`。
