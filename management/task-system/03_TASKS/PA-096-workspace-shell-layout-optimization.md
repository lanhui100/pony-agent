# PA-096 前端工作台布局优化（trace/metrics 二级化 + 配置页 tab 化）

## Basic Info

- ID: PA-096
- Status: Done
- Priority: P1
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-22
- Updated At: 2026-08-22
- OpenSpec Change: `openspec/changes/archive/2026-08-22-workspace-shell-layout-optimization/`（2026-08-22 归档，canonical spec 落于 `openspec/specs/workspace-shell-navigation/spec.md`）
- Spec 状态: spec 双路对抗审核完成并采纳修订（2026-08-22：架构 PASS WITH REVISIONS 2×P0 拆分线修正 + UX PASS WITH REVISIONS 产品决策升级用户拍板）；实现后双 code reviewer 审核均 PASS WITH REVISIONS（无 P0/P1，P2×4 全部修复）

## Background

对话页右侧栏同时承载状态、Tools、Trace、Plan、Debug 五段。Trace 是工程化调试数据（turn timeline、provider call、build context 证据），体量大，与"对话"主心智冲突；模型监控（metrics）与 trace 同属遥测却是独立一级页面；工具目录是配置信息却占据右侧栏；左侧栏把 provider/model 配置藏在"模型管理"二级折叠组；配置功能增多后单一纵向页不可扩展。

## Goal（对应用户需求 1–5）

1. Trace 移出右侧栏 → 二级遥测页（TelemetryPage·Trace tab），入口=左栏一级键（coding 显"遥测"/work 显"指标"，用户决策①）。
2. 右侧栏仅保留 状态/Plan/Debug；Tools 目录移入配置页第三个 tab。
3. 左侧栏：工作区区段第一优先；移除"模型管理"折叠组；"模型配置"升为一级菜单直达配置页模型 tab。
4. 模型监控并入同一遥测页第二个 tab（ModelMonitorPage 仅加 embedded 样式 prop）。
5. 配置页改为 tab 布局：通用 / 模型 / 工具。
6. 门禁口径（用户决策②）：trace tab coding 专属；指标 tab 双模式可见；tab 可用性收敛替代 App 级守卫。

## Scope

- 修改：`src/App.vue`、`src/components/HomeSidebar.vue`、`src/components/HomeSessionSidebar.vue`
- 小改：`HomeTracePanel.vue`（+expanded）、`ModelMonitorPage.vue`（+embedded）
- 新增：`src/lib/runtime/useTraceProjection.ts`（useTraceProjection/useCopyFeedback/纯函数单源复用 trace.ts）、`telemetry/TelemetryPage.vue`、`TraceInspector.vue`、`config/{ConfigPage,ConfigGeneralSection,ConfigToolsSection}.vue`、`types/config.ts`
- 删除：`SettingsPanel.vue`、`HomeToolsPanel.vue`
- 测试：App.spec 改写 + HomeSessionSidebar.spec 更新 + HomeSidebar.spec 拆分（26 条零丢失：18 trace→TraceInspector.spec、2 工具→ConfigToolsSection.spec、4 状态保留、2 折叠语义被自动展开语义取代）+ 新增 ConfigPage.spec/TelemetryPage.spec
- 配套：`vite.config.ts`/`vitest.config.ts` 改 import.meta.url 推导目录（支持 `--configLoader native` 进程内加载，沙箱环境无需 spawn 子进程）

## Non-Goals

- 不改 Pinia store 数据流、不改 Rust 后端、不引入 vue-router
- 不重设计 ModelMonitorPage / ProviderConfigPage 内部交互

## Acceptance Criteria

1. HomeSidebar 渲染不含 `tools-panel-toggle`/`trace-panel-toggle`（有测试）
2. 遥测页 coding 双 tab 默认 Trace；work 单指标 tab 且页头文案随模式切换（有测试）；启动竞态 tab 收敛有回归测试；work→coding 往返停留指标有显式断言
3. 返回按钮 emit navigate("home")（有测试）
4. 左侧栏 DOM 顺序 actions→工作区→对话→底部一级导航；工作区头部显示激活工作区名；collapsed rail 六键（有测试）
5. sidebar emit "models"/"settings" 打开 config 对应 tab；configTab 会话内受控态不持久化（实现修订，YAGNI）；tools tab 高亮 null（有测试）
6. vitest 全量绿 + `npm run build` 通过 ✅（最终：434 passed / 10 skipped，vue-tsc 0，build ✓）

## Review Plan & Records

spec 双路对抗审核（架构 + UX/可测性）→ 采纳修订 → 实现 → 双 code reviewer 对抗审核 diff → 测试门禁 → 收口归档

### Spec 审核采纳记录（2026-08-22）

- 架构（PASS WITH REVISIONS，2×P0）：①聚合 computed/copyText 是状态面板供给——不得整体搬入 TraceInspector，抽 `useTraceProjection`/`useCopyFeedback` 共享管线【已落实】；②liveTraceTurn 冻结守卫保留 + open 初始 false→mounted nextTick true 首滚补偿【已落实】；测试按交互 testid 分类迁移、noUnusedLocals 解构修剪、高度契约显式化、memo 安全依赖 out-in 写入 design【均已落实】
- UX（PASS WITH REVISIONS，P0 同架构 P0-1 + P1×5/P2×5）：遥测入口可达性与 metrics 门禁升级用户拍板【用户决策：仅左栏一级键；work 仅见指标】；embedded prop 解决双壳冲突【已落实】；APG tabs 完整规格【已落实】；操作行徽章去重【已落实】；openspec specs delta 补齐【已创建并归档】

### 实现 Code Review 采纳记录（2026-08-22，双路均 PASS WITH REVISIONS）

- A-P2-1 提交卫生：`.gitignore` 补 `.pnpm-store/` 与 `NUL.*`；无关的 Cargo.lock/docs 改动不入本次范围（提交时圈定 src/tests/openspec/management 文件）
- A-P2-2 = B-P3-2：ConfigPage/types/config.ts 过期持久化注释纠正【已修】
- B-P2-1 TelemetryPage 页头 work 下仍自称"遥测"/含 Trace 字样 → 标题/描述随 isCoding 切换【已修 + 测试】
- B-P2-2 Trace 复制按钮键盘不可达（预存在但暴露面扩大）→ 四处补 `group-focus-within:visible`【已修】
- A-P3-3/B-P3-3 死导出与重复函数 → useTraceProjection 收敛返回面（latestTurn/turnTimelineCache 内部化）+ cacheHit 函数单源复用 trace.ts【已修 + design.md 同步】
- B-P3-1 ConfigPage focusTab 双 rAF → 同步聚焦统一两页模式【已修】
- B-P3-10 expanded 头部 shrink-0【已修】
- 测试补强（review 建议）：冻结聚合守护、expanded 类契约、work→coding 往返落点、双页 APG 键盘漫游【全部新增，+6 用例】
- 接受残余（P3，登记不阻塞）：memo 卸载不清（既有模式，out-in 保证不同址）；rail 部分 aria-label 仅 title 兜底；disclosure 控件无 aria-expanded（预存在模式）；workspace 表单取消不清错误；cancelEditExa 无 catch（自 SettingsPanel 继承）；embedded 指标页 tab 往返重拉 IPC；config 页无返回钮（与旧持平）；e2e 未覆盖导航矩阵

## Current Progress

- 2026-08-22：立卡；openspec change 四件套；spec 双路对抗审核采纳修订；用户产品决策两项拍板
- 2026-08-22：实现完成（组件/App/测试全量）；门禁全绿（vitest 428→434 通过、vue-tsc 0、build ✓）
- 2026-08-22：实现后双 code reviewer 审核，P2×4 + P3 采纳项全部修复复验通过；openspec change 归档 + canonical spec 落地
- Next Action：无（收口完成）。提交建议：圈定 src/ tests/ openspec/ management/ vite.config.ts vitest.config.ts .gitignore，不含无关 Cargo.lock 与 docs/learning 改动
- Resume Hint：后续优化清单见任务卡 Review Plan & Records「接受残余」节
