# 0009 工作台信息架构：trace/metrics 二级遥测页与配置页 tab 化

Status: superseded by [0013](../0013-workbench-nav-observation-entry-and-provider-hierarchy.md)（观测入口移右栏浮动按钮、"模型配置"一级键删除；其余决定由 0013 承接重述）

## 背景

PA-096 之前，对话页右侧栏同时承载状态、Tools、Trace、Plan、Debug 五段：Trace 是工程化调试数据（turn timeline、provider call、build context 证据），体量大且与"对话"主心智冲突；模型监控（metrics）与 Trace 同属遥测却占据独立一级页面；工具目录是配置信息却常驻对话页；provider/model 配置藏在左侧栏"模型管理"二级折叠组内。行为契约以 `openspec/specs/workspace-shell-navigation/spec.md` 为准。

## 候选方案

**遥测入口位置**

- 右侧栏底部按钮（spec 初稿）：落选——窗口 <1000px 时右栏被强制隐藏且浮动开关一并消失，窄窗口下遥测零入口（UX 对抗审核 P1）。
- 右栏按钮 + 左栏兜底键双入口：落选——双入口增加导航心智负担，用户裁决保持单一入口。
- **仅左侧栏一级键（采纳）**：任何窗口宽度可达；存在感略高于"二级"定位，用户知情接受。

**metrics 门禁口径**

- 与 trace 同门禁（work 完全不可见）：落选——token/成本/hook 聚合对写作分析场景同样有价值，且原始需求只限定 trace 为 coding 专属。
- 双模式全可见（含 trace）：落选——需求明示 work 不显示 trace。
- **work 仅见"指标"tab、trace tab coding 专属（采纳）**：布局一致、门禁分级；模式切换的启动竞态由 tab 可用性收敛处理，不引入 App 级弹回守卫。

**trace 承载形态**

- 抽屉/drawer 保留对话上下文：落选——长 timeline 阅读受益于全宽，浮层滚动与对话流互相锁死。
- **整页二级切换（采纳）**：进入即聚焦读面，返回路径单一（遥测页返回按钮回 home）。

**trace 数据管线拆分线**

- 会话级聚合 computed 随 TracePanel 整体迁入新组件：落选——聚合是右侧状态面板 13 个 props 中 9 个的唯一数据源、copyText 为状态面板与 trace 双消费，整体搬迁导致状态面板指标归零与复制静默失效（实现前架构对抗审核 P0 拦截）。
- HomeSidebar 保留全链、新组件接收 turns prop 直传：落选——两份相同投影链在流式期间重复计算。
- **抽 `useTraceProjection` 共享 composable（采纳）**：冻结守卫参数化为 `liveTurnEnabled`（PA-086 语义保留）；HomeSidebar 以恒 false 消费（与旧"面板折叠时聚合不含进行中 turn"逐位一致），TraceInspector 以 open 消费。

**configTab 持久化**

- localStorage 持久化 + 非法值回退：落选——"模型配置/设置"两个一级键本身是显式目的地，应用启动后总从 home 开始，持久化值只写不读（死状态）。
- **会话内受控态（采纳）**：ConfigPage 对非法 prop 值防御性回退通用 tab。

## 决策

工作台采用三级信息架构：对话页右栏只保留对话过程面板（状态/计划/调试）；Trace 与 metrics 收敛为遥测页双 tab（Trace coding 专属），经左栏一级键进入；工具目录并入配置页第三个 tab；配置页 tab 化为受控态容器（无 vue-router，维持 `currentPage` 条件渲染）；左侧栏工作区区段置顶、"模型配置"升一级菜单直达配置页模型 tab。

## 影响

- 左栏冻结边界规则更新：原"可观测性信息默认进右栏"约定不再覆盖 trace/metrics（注记见 `docs/guides/frontend-layout-and-observability-boundary.md`）。
- `HomeTracePanel` 新增 `expanded` prop、`ModelMonitorPage` 新增 `embedded` prop，是二者仅有的对外扩展点；侧栏/整页历史用法不变。
- 后续 IA 调整须走 `workspace-shell-navigation` spec 的 OpenSpec 变更流程；相关任务卡 PA-096 记录完整审核账目。
