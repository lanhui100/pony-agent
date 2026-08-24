# Tasks: workspace-nav-observation-entry-and-provider-hierarchy

## 1. 导航壳层

- [x] 1.1 `src/types/config.ts`：`SidebarNavigationPage` 收窄为 `"home" | "settings"`
- [x] 1.2 `src/App.vue`：`handleSessionNavigate` 删 models/telemetry 分支；`sessionSidebarActivePage` 显式映射（config∧general→settings，telemetry→null）；右栏右上角新增观测浮动按钮（Tooltip"观测"、aria-label、focus ring、<1000px 落位 right-3）
- [x] 1.3 `src/components/HomeSessionSidebar.vue`：删除遥测/模型配置两入口（展开+折叠态）；清理 Activity/Server/Settings2/useSettingsStore/workspaceMode 残链

## 2. 观测更名

- [x] 2.1 `TelemetryPage.vue`：页头统一"观测"；副标题去"不随对话页常驻"；tablist aria-label 更名
- [x] 2.2 `ConfigGeneralSection.vue`：工作模式卡片描述改写（Trace 观测读面 / 观测仅保留指标）
- [x] 2.3 `ModelMonitorPage.vue`：内嵌头"模型监控"→"指标监控"

## 3. 提供商页层次折叠

- [x] 3.1 左列扁平化：去 openProviderId/toggleProvider 手风琴，点击即选中 + 高亮
- [x] 3.2 右侧双一级折叠区渲染矩阵：提供商详情三态+动作条下沉；模型列表行内展开详情、新增表单卡、行级动作条；折叠锁与行禁点规则
- [x] 3.3 页头收敛为当前提供商名；begin* 统一收口 expandedModelId/modelCreateOpen

## 4. 测试

- [x] 4.1 `tests/App.spec.ts`：stub 收窄、Tooltip stub、settings 导航用例、models/tools 高亮空断言、观测按钮开页/返回、960px 常显
- [x] 4.2 `tests/HomeSessionSidebar.spec.ts`：底部仅设置、折叠栏四键、守卫用例改写、角标用例更新
- [x] 4.3 `tests/TelemetryPage.spec.ts`：coding/work 页头"观测"断言
- [x] 4.4 新增 `tests/ProviderConfigPage.spec.ts`（7 例：扁平列表/双 section 折叠/行内详情/新增模型/隐藏列表/编辑锁折/删除回落）

## 5. 文档与归档

- [x] 5.1 ADR 0013（implemented，含承接重述节）；0009 → superseded/ 并互链；README 索引更新
- [x] 5.2 `docs/guides/frontend-layout-and-observability-boundary.md` 注记增补 ADR 0013 修订
- [x] 5.3 本变更 specs delta 合入正典 spec，变更目录移入 `openspec/changes/archive/`

## 6. 门禁

- [x] 6.1 定向 vitest（5 个受影响文件）
- [x] 6.2 全量 vitest：31 文件 / 511 passed / 10 skipped / 0 failed
- [x] 6.3 `npm run typecheck` 通过
- [x] 6.4 `npm run test:ui-guard`：branches 78.99% —— 存量失败延续（PA-099 取证 HEAD 基线 78.95% 同不达 80%，拖累源为本次未改动的 HomeWorkspace.vue 75.76%；本次对两受测文件的分支增删基本中性）。沙箱运行需 `--configLoader native --pool=threads`。

## 7. 实施迭代（同日用户反馈）+ 双审回改

- [x] 7.1 观测按钮与折叠按钮同规格（App.spec 加一致性断言）
- [x] 7.2 两折叠区开关改为头部居中纯图标 chevron（左栏折叠按钮同族）；锁定说明挂外层 span title
- [x] 7.3 编辑/删除收敛为纯图标 + tooltip（side=top，保留 ghost hover 底色）
- [x] 7.4 模型视图单列纵排、去"所属提供商"、能力仅图标带 tooltip、参数 K/M 单位 + 常规值徽标一键填入（去 tokens 字样）
- [x] 7.5 双审修复：P1-1 提供商区体 isEditingProvider 守卫；P1-2 beginCreate/EditProvider 收口 providerSectionOpen；spec 正典两处漂移回填；notice/error 上提；行 disclosure aria-controls；provider 行 focus ring
- [x] 7.6 测试扩至 ProviderConfigPage.spec 12 例（P1×2 回归、折叠锁/行锁、图标动作、K/M 徽标填充）＋ App.spec 一致性断言；门禁复跑全绿

## 8. 实施迭代二（同日用户反馈）

- [x] 8.1 折叠钮移至标题左侧垂直居中（一级区）；模型行 chevron 归入行尾动作簇
- [x] 8.2 行尾动作簇：编辑/删除移至模型行尾不再独占一行；新增模型改纯图标+tooltip；全部行尾图标缩小弱化（h-7 w-7 / stone-400 / 透明底），hover 暖橙增强；空闲态门控 modelActionsIdle 统一，编辑期行尾动作整体移除
- [x] 8.3 折叠动画平滑化：grid-rows 0fr↔1fr 过渡容器 + overflow-hidden + motion-reduce 降级，内容常挂载，折叠态以 data-open/aria-hidden 表达
- [x] 8.4 removeCurrentModel → removeModelById(providerId, modelId) 支持行尾直删；孤儿 computed（canDeleteModel/activeModelRowId）清理
- [x] 8.5 spec 重写至 13 例（动画态 data-open 断言、行尾直删、锁定的 v-if 移除语义）；typecheck + 受影响 spec 全绿

## 9. 实施迭代三（交互 bug 修复）

- [x] 9.1 两一级区改手风琴互斥（默认提供商详情展开；begin* 自动切换目标区并收起另一侧）
- [x] 9.2 全层级整行 trigger 折叠：hover 背景/pointer 包裹含行尾动作的整行；动作区 @click.stop；chevron 纯指示器（无独立 hover 底色）
- [x] 9.3 修复一级折叠不生效：grid-template-rows 行高改内联 style 表达，不再依赖 Tailwind 任意值类
- [x] 9.4 锁定语义行为化（守卫拦截而非 disabled 属性）；spec 扩至 14 例全绿；typecheck 通过

## 10. 实施迭代四（排版密度）

- [x] 10.1 一级区头 hover 仅 pointer、移除背景色
- [x] 10.2 左列提供商行：协议/计数徽标与名字同行尾部显示
- [x] 10.3 读面紧凑化：字段名+值同行（dl 两栏，长值跨栏）、协议入口单行化、能力输入/输出并排；卡片 py 与表单行距减半；编辑表单简单字段内联标签（两份副本同步）
