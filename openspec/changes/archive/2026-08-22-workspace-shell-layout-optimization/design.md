# Design: workspace-shell-layout-optimization

> 本版已合并：架构审核修订（PA-096-R1，P0×2/P1×4）、UX 审核修订（PA-096-R2）、
> 用户产品决策（2026-08-22）：①遥测入口 = 左侧栏一级键；②work 模式仅见"指标" tab。

## 目标信息架构

```
Shell
├─ 左侧导航栏 HomeSessionSidebar
│   ├─ 品牌（→ home）+ 折叠开关
│   ├─ 操作行：新对话（全宽；原激活工作区徽章移除——与工作区 section 头去重）
│   ├─ 工作区 section（第一优先；头部右侧显示激活工作区名）
│   ├─ 对话 section（按 workspace 分组会话树，不变）
│   └─ 底部一级导航（纵向）：
│       ├─ 遥测/指标（Activity 图标；coding 显"遥测"、work 显"指标"；→ telemetry 页）
│       ├─ 模型配置（Server 图标；→ config·models tab）
│       └─ 设置（Settings 图标；→ config·general tab）
├─ 主内容区（currentPage 切换，Transition out-in 保留）
│   ├─ home：HomeWorkspace + 右侧栏 HomeSidebar
│   ├─ config：ConfigPage（tabs: 通用 / 模型 / 工具）
│   └─ telemetry：TelemetryPage（coding: Trace+指标 双 tab；work: 仅指标单 tab）
└─ 右侧栏 HomeSidebar（home 页内，两种模式均显示）
    ├─ 状态 HomeStatusPanel（保留）
    ├─ Plan PlanPanel（保留）
    └─ Debug DebugPanel（保留）
```

## 路由与状态模型（App.vue）

- `type AppPage = "home" | "config" | "telemetry"`（移除 `"providers" | "model-monitor" | "settings"` 页面值）
- `type ConfigTab = "general" | "models" | "tools"`；`configTab` 为**会话内受控状态**
  （实现修订：不持久化——两个一级键本身是显式目的地，应用启动后总从 home 开始，
  持久化该值只会产生只写不读的死状态；ConfigPage 对非法 prop 值防御性回退 general）
- 左侧栏 `navigate` emit（`"home" | "models" | "settings" | "telemetry"`）映射：
  - `"home"` → currentPage=home
  - `"telemetry"` → currentPage=telemetry
  - `"models"` → currentPage=config, configTab=models
  - `"settings"` → currentPage=config, configTab=general
- **无 App 级 work 守卫**（用户决策②的推论）：work 下遥测页仍可达但只渲染指标 tab；
  启动竞态（默认 coding → 进入 Trace tab → loadSettings resolve 为 work）由 TelemetryPage 的
  tab 可用性收敛处理（见下），不再弹回 home
- 传给左侧栏的 `currentPage` prop 为派生高亮态：
  - home → `"home"`；telemetry → `"telemetry"`
  - config+models → `"models"`；config+general → `"settings"`
  - config+tools → `null`（工具无对应一级键，不点亮任何项——有意设计，避免"工具≠设置"心智错位）

## 共享数据管线（架构审核 P0-1 / P0-2 修订）

新增 `src/lib/runtime/useTraceProjection.ts`：

### useTraceProjection({ liveTurnEnabled })

- 输入：`liveTurnEnabled: Ref<boolean>` —— 原 `activePanel === "trace"` 冻结守卫的参数化。
  守卫表达式 `if (!liveTurnEnabled.value) return null` 必须保留（PA-086 约束：禁用时活跃 turn
  不并入 orderedTurnTraces，切断与 store 可变数组的引用共享、避免 memo 陈旧命中）
- 返回：`orderedTurnTraces`、`sessionTurnCount`、`sessionModelCallCount`、
  `sessionToolCallCount`、`sessionInputTokensTotal`、`sessionCacheHitTokensTotal`、
  `sessionOutputTokensTotal`、`sessionCacheHitRatio`、`contextDisplayTokens`、
  `showContextUsage`、`currentContextWindowTokens`
  （review B-P3-3 收敛：`latestTurn`/`turnTimelineCache` 仅作内部派生源，不导出）
- 消费方：
  - **HomeSidebar**：`liveTurnEnabled = ref(false)`（常量）——与原"trace 面板折叠时聚合不含进行中
    turn"的行为逐位一致，HomeStatusPanel 13 个 props 中 9 个的数据源由此供给
  - **TraceInspector**：`liveTurnEnabled = open`

### 纯函数与剪贴板反馈

- `canonicalTraceTimelineKind`：直接复用 `trace-projection.ts` 已有导出，删除 HomeSidebar 内本地重复实现
- `providerReturnedCacheHitInputTokens(turn)`：单源复用 `trace.ts` 的
  `resolveProviderReturnedCacheHitInputTokens`（review B-P3-3：消除逐字节重复），本模块转导出
- `useCopyFeedback()`：`copiedKey` / `copyText` / 定时器清理。两个消费方**各自实例化**
  （key 命名空间天然隔离："session-id" vs trace keys，行为差异无害）

### memo 全局性说明

`clearTraceProjectionMemo` 为模块级全局清理；安全性依赖 App Transition `mode="out-in"` 保证
HomeSidebar 与 TraceInspector 永不同时挂载（清理只引发重算）。**若未来改为并行 Transition 需重估**。
sessionId watch 清理逻辑保留在两处消费方各自内部。

## 组件设计

### TraceInspector.vue（新增，自包含）

- 消费 `useTraceProjection({ liveTurnEnabled: open })` + 自有 `useCopyFeedback`
- **open 初始 false；onMounted 后 nextTick 置 true**（架构审核 P1-1/P1-2 修订）：
  ①挂载瞬间守卫生效（冻结语义）；②触发 HomeTracePanel 内部 `open false→true` watch，
  正常执行首次 scroll-to-bottom（用户落点 = 最新 turn 而非 timeline 顶部）
- 模板：根 `<section class="flex h-full min-h-0 flex-col">` + `HomeTracePanel :expanded="true"`
  （props 与原调用一致 + expanded）；`@toggle` 折叠为 header-only（守卫随之冻结数据）
- `sessionId` watch 清理 projection memo 与 copiedKey；`onBeforeUnmount` 清理 copy timer
- 不含状态/工具/Plan/Debug

### HomeTracePanel.vue（唯一修改点：expanded 可选 prop）

- `expanded?: boolean`（默认 false，侧栏历史用法零变化，既有测试不受影响）
- expanded=true 时：外层 section 改 flex column 填满父容器（去掉 border-b/pb-4 侧栏装饰）；
  body 容器改 `flex min-h-0 flex-1 flex-col`；内嵌 ScrollArea 高度类从
  `max-h-[24rem] min-h-[3rem]` 切换为 `min-h-0 flex-1`（消除全页场景"大片空白+小窗滚动"）

### TelemetryPage.vue（新增）

- emits：`navigate("home")`（返回按钮）；tabs 数据自读 settings store：
  - coding：`[Trace, 指标]`，默认 trace
  - work：`[指标]` 单 tab（原始调试明细不可达）
- **tab 可用性收敛**：watch 可用 tab 列表，当前 activeTab 不在列表 → 自动切到首个可用 tab
  （覆盖启动竞态 coding→work 时正停留在 Trace 的窗口）
- 结构：flex column 全高壳（头部 shrink-0：返回按钮 + 标题；tab 条 shrink-0；body `min-h-0 flex-1`）+
  body `v-if` 懒挂载：Trace → `TraceInspector`；指标 → `ModelMonitorPage :embedded="true"`

### ModelMonitorPage.vue（唯一修改点：embedded 可选 prop）

- `embedded?: boolean`（默认 false：现页面壳/标题/刷新按钮行为不变，既有测试不受影响）
- embedded=true 时：根节点去卡片壳（rounded/border/px/py）改纯 flex column；隐藏 eyebrow+h2+描述块，
  保留刷新按钮行（`model-monitor-refresh` testid 不变）；内部滚动区依赖宿主提供的定高 flex 链

### ConfigPage.vue（新增）

- props：`tab: ConfigTab`；emits：`update:tab`
- 结构：卡片 shell（rounded-[0.6rem] border bg-white/72，与右栏视觉一致）+ 头部（标题"配置"）+
  tab 条 + body `min-h-0 flex-1`（各 tab body v-if 懒挂载，body wrapper 提供 h-full 链）：
  - general → `ConfigGeneralSection`
  - models → `ProviderConfigPage`（原样复用，其 `grid h-full` 根直接填满 body）
  - tools → `ConfigToolsSection`

### ConfigGeneralSection.vue（新增）

原 `SettingsPanel.vue` 内容（工作模式双卡 + EXA 密钥编辑行），去掉外层卡片壳与"配置"标题
（由 ConfigPage 统一承担），业务逻辑零改动。

### ConfigToolsSection.vue（新增）

- 读 `runtimeStore.availableTools`，静态列表：中文名优先、kind、描述、权限/审批/来源摘要
  （文案规则自 `HomeToolsPanel.vue` 移植——非"零修改复用"，proposal 已更正表述）
- 空态："暂无可用工具"；testid 前缀 `config-tools-*`

### HomeSidebar.vue（瘦身）

- 移除：HomeToolsPanel、HomeTracePanel 引用；trace 呈现逻辑（由 composable 承接后删除本地副本）
- 修剪 `storeToRefs` 解构至状态面板最小集（noUnusedLocals 门禁：`availableTools/traceSteps/
  toolActivities/provider*/totalTokens/firstTokenLatencyMs/messages/phase` 等失去消费者的项必须删除）
- 保留：HomeStatusPanel（聚合 props 来自 `useTraceProjection({ liveTurnEnabled: ref(false) })`）、
  PlanPanel、DebugPanel 及 activePanel（收窄为 `"plan" | "debug" | ""`）
- **不再有遥测入口**（入口移至左侧栏一级键——用户决策①）

### HomeSessionSidebar.vue（重排）

- ScrollArea 内顺序：工作区 section → 对话 section
- 工作区 section 头部右侧显示激活工作区名（截断 + stone-400 小字）；操作行徽章删除（去重）
- 移除"模型管理"折叠组（`modelOpen`、`MODEL_OPEN_STORAGE_KEY`、`toggleModelSection` 删除）
- 底部 nav 三项（纵向，menuInteractiveClass + selected 态）：
  - `session-sidebar-nav-telemetry`：coding 显"遥测"/work 显"指标"，Activity 图标，emit("telemetry")
  - `session-sidebar-nav-providers`：Server 图标，文案"模型配置"，emit("models")（一级菜单）
  - `session-sidebar-nav-settings`：不变，emit("settings")
- collapsed rail 六键：brand / new-chat / home / telemetry / providers / settings
- `NavigationPage = "home" | "models" | "settings" | "telemetry"`（emit 与高亮同集合——入口在左栏后类型污染消失）；
  prop 类型放宽为 `NavigationPage | null`

## 可访问性

- ConfigPage 与 TelemetryPage tabs 按 WAI-ARIA APG：`role="tablist"` / `role="tab"` /
  `aria-selected` / `aria-controls` ↔ `role="tabpanel"` + `aria-labelledby` 配对；
  roving tabindex + ArrowLeft/ArrowRight/Home/End 键位漫游
- 页面切换焦点管理：TelemetryPage/ConfigPage 根标题 `tabindex="-1"`，onMounted 聚焦，
  避免 out-in 后焦点丢到 body（WCAG 2.4.3）
- 返回按钮带可访问名（aria-label="返回对话"）；左侧栏一级键均有可见文案（展开态）

## 兼容与迁移说明

- localStorage：无新增键（configTab 不持久化——见路由与状态模型一节的实现修订）；
  废弃键 `pony-agent.session-sidebar-model-open.v1` 无读端残留无害（已核实全库引用）
- 右侧栏开合持久化（`pony-agent.ui.right-sidebar-open`）、宽度断点行为不变
- E2E 影响评估：`tests/e2e/browser-preview.spec.ts` 与 `tests/e2e/checkpoint-branch.spec.ts`
  用到的 testid（session-sidebar-session-list / home-right-sidebar-shell / session-sidebar-new-chat /
  workspace-* 等）全部保留，预期不需改动；注意 e2e 从未覆盖页面导航，不能作为本次导航回归的安全证据
- 返回路径不对称（已知取舍）：遥测/config → 只能回 home；记录于任务卡

## 回滚

单点前端变更，无数据迁移；`git revert` 即可完整回滚。

## 验证策略

1. `npx vitest run`（全量单测）
2. `npm run build`（vue-tsc --noEmit + vite build）
3. 手工冒烟（浏览器预览模式）：coding/work × home/config(3 tab)/telemetry(tab 收敛) 矩阵；
   遥测页 trace 视口全高观感确认
