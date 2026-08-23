# Proposal: workspace-shell-layout-optimization

## Why

对话页右侧栏当前同时承载五段内容（状态、Tools、Trace、Plan、Debug）。其中：

- **Trace** 是工程化调试数据（turn timeline、provider call 记录、build context 证据），体量大、交互重，与"对话"主心智冲突，不应与聊天同屏常驻；
- **Tools 目录** 是纯展示型配置信息（可用工具与权限摘要），属于配置范畴而非对话过程信息；
- **模型监控页**（metrics）与 Trace 同属遥测（telemetry），却是独立一级入口，与 trace 的入口割裂，且同样不该占据一级导航心智；
- **左侧栏** 把 provider/model 配置藏在"模型管理"二级折叠组内，层级与使用频率不匹配；
- **配置功能**（工作模式、服务密钥、provider/model、工具目录）增多后，单一纵向滚动页不可扩展。

## What Changes

信息架构调整（纯前端，`src/` 层）：

1. **Trace 二级化 + coding 专属**：右侧栏移除 Trace 折叠段；新增二级遥测页（`TelemetryPage`），入口为左侧栏一级键（用户决策：仅左栏键，任何窗口宽度可达）；coding 模式下遥测页含 Trace + 指标双 tab；work 模式不渲染 Trace tab（原始调试明细不可达），也不在左侧栏显示"遥测"字样（同键位显示"指标"）。
2. **Tools 移入配置页**：右侧栏移除 Tools 段；工具目录成为配置页第三个 tab；`HomeToolsPanel.vue` 删除（文案规则移植至新 section）。
3. **右侧栏保留**：状态（HomeStatusPanel）、Plan（PlanPanel）、Debug（DebugPanel）三段不变（对需求"头部的对话状态、plan、debug 保留"的解读：保留在右侧栏原位，不搬家）。
4. **左侧栏重排**：工作区区段提升为第一优先（先于会话列表），激活工作区名并入 section 头部（操作行徽章去重）；移除"模型管理"折叠组；"模型配置"升为一级菜单项直达配置页"模型" tab。
5. **Metrics 与 trace 同布局**：模型监控并入同一二级遥测页作为 tab（ModelMonitorPage 仅加 `embedded` 样式 prop，业务零修改）。门禁口径（用户决策）：布局一致、门禁分级——trace tab coding 专属，指标 tab 双模式可见（work 场景的 token/成本统计仍有价值；原始 turn 调试明细仍限 coding）。
6. **配置页 tab 化**：新增 `ConfigPage`（tabs：通用 / 模型 / 工具），通用 tab 承载原 `SettingsPanel` 内容（工作模式 + 服务密钥），模型 tab 承载原 `ProviderConfigPage`，工具 tab 为新的工具目录列表。tab 为会话内受控状态（一级键即显式目的地，不做跨重启持久化；实现修订 2026-08-22）。`SettingsPanel.vue` 删除。

## Scope

- 修改：`src/App.vue`、`src/components/HomeSidebar.vue`、`src/components/HomeSessionSidebar.vue`
- 小改：`src/components/HomeTracePanel.vue`（+`expanded` 可选 prop）、`src/components/ModelMonitorPage.vue`（+`embedded` 可选 prop）
- 新增：`src/lib/runtime/useTraceProjection.ts`、`src/components/telemetry/TelemetryPage.vue`、`src/components/TraceInspector.vue`、`src/components/config/{ConfigPage,ConfigGeneralSection,ConfigToolsSection}.vue`
- 移除：`src/components/SettingsPanel.vue`、`src/components/HomeToolsPanel.vue`
- 测试：改写 `tests/App.spec.ts`、`tests/HomeSessionSidebar.spec.ts` 导航相关用例；拆分 `tests/HomeSidebar.spec.ts`（trace 用例迁 `tests/TraceInspector.spec.ts`，工具用例迁 `tests/ConfigToolsSection.spec.ts`，按交互 testid 而非用例名分类）；新增 `tests/ConfigPage.spec.ts`、`tests/TelemetryPage.spec.ts`

## Non-Goals

- 不改 runtime/providers/plan 等 Pinia store 的数据流与命令
- 不改 Rust 后端与 Tauri command
- 不引入 vue-router（维持现有 `currentPage` 切换 + Transition 模式）
- 不做 ModelMonitorPage / ProviderConfigPage 内部的 UX 重设计
- 不修既有 collapsed rail 图标按钮仅有 title 的可访问性欠账（本次未恶化；记录为后续项）
- vitest coverage.include 清单与 `test:ui-guard` 脚本文件集不变（新组件暂不入覆盖率测量；记录为后续项）

## Task Tracking

`management/task-system/03_TASKS/PA-096-workspace-shell-layout-optimization.md`（Complexity B）
