# Tasks: workspace-shell-layout-optimization

## 1. 数据管线与组件

- [ ] 1.1a 新增 `src/lib/runtime/useTraceProjection.ts`：`useTraceProjection({ liveTurnEnabled })`（守卫 `if (!enabled) return null` 显式保留）、`providerReturnedCacheHitInputTokens` 纯函数、`useCopyFeedback()`；canonical kind 复用 trace-projection.ts
- [ ] 1.1b 新增 `src/components/TraceInspector.vue`：消费 composable；open 初始 false → onMounted nextTick 置 true（首滚补偿）；sessionId watch 清 memo/copiedKey；unmount 清 copy timer
- [ ] 1.1c `HomeTracePanel.vue` +`expanded` 可选 prop（默认 false 行为不变）
- [ ] 1.2 新增 `src/components/telemetry/TelemetryPage.vue`：coding 双 tab / work 单指标；tab 可用性收敛 watch；返回按钮；APG tabs a11y；根标题聚焦
- [ ] 1.2c `ModelMonitorPage.vue` +`embedded` 可选 prop（默认 false 行为不变）
- [ ] 1.3 新增 `src/components/config/ConfigPage.vue`：卡片 shell + APG tabs + body 高度链
- [ ] 1.4 新增 `src/components/config/ConfigGeneralSection.vue`（原 SettingsPanel 内容去壳）
- [ ] 1.5 新增 `src/components/config/ConfigToolsSection.vue`（工具目录静态列表 + 空态）
- [ ] 1.6 瘦身 `HomeSidebar.vue`：移除 Tools/Trace 段与本地 trace 副本；聚合改由 `useTraceProjection({ liveTurnEnabled: ref(false) })` 供给；**修剪 storeToRefs 解构至状态面板最小集**（noUnusedLocals）
- [ ] 1.7 重排 `HomeSessionSidebar.vue`：工作区 section 置顶 + 头部显示激活工作区名、操作行徽章删除、移除模型管理组、底部一级导航三项（遥测|指标 / 模型配置 / 设置）、collapsed rail 六键
- [ ] 1.8 App.vue：AppPage 收敛 home/config/telemetry；configTab 会话内受控状态（实现修订：不持久化）；navigate 映射；派生高亮态（config+tools→null）
- [ ] 1.9 删除 `src/components/SettingsPanel.vue`、`src/components/HomeToolsPanel.vue`

## 2. 测试

用例迁移分类规则：**按交互 testid 归类**（`tools-panel-toggle`→工具目录；`trace-step-*`/`trace-panel-toggle`/turn 摘要→trace），不按用例名称。

### tests/HomeSessionSidebar.spec.ts

- [ ] 2.1 排序断言改为 actions → 工作区 section → 对话 section → 底部导航；工作区 section 头部含激活工作区名
- [ ] 2.2 导航断言更新：`session-sidebar-nav-telemetry`（coding 显"遥测"/work patch store 后显"指标"）emit("telemetry")；`session-sidebar-nav-providers` emit("models")；settings 不变
- [ ] 2.3 collapsed rail 六键断言（brand/new-chat/home/telemetry/providers/settings）；删除 model-management 相关用例（L638"toggles model management"整条删除，原因：被测语义消亡）
- [ ] 2.4 mountSidebar helper 的 currentPage 字面量更新（"model-monitor"→"telemetry"）

### tests/HomeSidebar.spec.ts（保留侧）

- [ ] 2.5 L366 改写："右侧栏只保留 状态/Plan/Debug，不再渲染 Tools/Trace 段"
- [ ] 2.6 status 用例保留（L498/757/900），mountSidebar helper 去掉 trace 展开步骤
- [ ] 2.7 L797 status 半边拆出为独立负断言用例："状态面板不暴露 build-context 细节"（接受 vacuous 弱化并在用例注释注明；trace 正断言迁 TraceInspector.spec）

### tests/TraceInspector.spec.ts（新建，承接 HomeSidebar.spec trace 用例）

- [ ] 2.8 迁移清单：L377/387 改写为"挂载后自动展开并定位最新 turn"+"header 可折叠且折叠后 body 卸载"；L510/585/693/797(trace 半边)/937/1023/1047/1133/1267/1336/1388/1441/1524/1598/1663/1785/1853/1925 整体迁移（仅改挂载目标与 helper）
- [ ] 2.9 共享 fixture 提取：TooltipStub/ScrollAreaStub/clipboard mock/trace 工厂函数放 `tests/helpers/trace-inspector.ts`
- [ ] 2.10 冻结引用语义回归用例：折叠态下 liveTraceTurn 为 null（对应 PA-086 约束）

### tests/ConfigToolsSection.spec.ts（新建）

- [ ] 2.11 迁移 L407/651（中文名优先+权限摘要；approvalMode 缺失回退 requiresApproval）+ 空态用例

### tests/TelemetryPage.spec.ts（新建）

- [ ] 2.12 coding 默认 Trace tab 且渲染 TraceInspector stub；work 仅"指标"单 tab
- [ ] 2.13 tab 可用性收敛：coding 下停 Trace → store patch 为 work → activeTab 自动落"指标"（启动竞态回归）
- [ ] 2.14 remount-freshness：mid-turn 离开再进入，timeline 反映最新流式进度
- [ ] 2.15 返回按钮 emit navigate("home")

### tests/ConfigPage.spec.ts（新建）

- [ ] 2.16 三 tab 渲染（models tab 内 ProviderConfigPage stub 可见）；tab 切换 emit update:tab；非法 tab prop 回退 general
- [ ] 2.17 （修订）持久化用例取消——configTab 改为会话内受控状态（只写不读的死状态，YAGNI），App 级仅保留派生高亮断言

### tests/App.spec.ts

- [ ] 2.18 stub 清单更新（SettingsPanelStub/ModelMonitorPageStub → ConfigPageStub/TelemetryPageStub）；导航用例改写：home↔config(models/general)、home↔telemetry、sidebar emit("tools-tab 高亮 null") 断言派生 currentPage prop
- [ ] 2.19 configTab 持久化往返用例（App 级）

## 3. 验证门禁（必须全绿）

- [ ] 3.1 `npx vitest run` 全量单测通过
- [ ] 3.2 `npm run build`（vue-tsc --noEmit && vite build）通过
- [ ] 3.3 手工冒烟：coding/work × 页面矩阵；遥测页全高视口观感

## 4. 收口

- [ ] 4.1 双 code reviewer 对抗审核 diff，采纳/不采纳记录入任务卡
- [ ] 4.2 更新任务卡状态、看板；openspec change 归档标记
