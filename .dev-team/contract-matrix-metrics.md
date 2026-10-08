# 观测指标页重构契约矩阵与架构设计 (Metrics Tab Redesign Contract Matrix)

## 1. 现状痛点审计 (Audit & Friction Points)
从用户视角与业内可观测性最佳实践（如 Datadog LLM Observability, LangSmith, Arize Phoenix, Grafana）审视，当前 `ModelMonitorPage.vue` 存在以下显著痛点：
1. **视觉信息过载与无序堆砌（Wall of Cards & High Entropy）**：
   - 现存页面将 Providers, Models, Tools, Hook Classes, Hooks, Capability Sources, Capability Invocation Modes, Capability Failure Classes, Skill Selections, Skill Sources, Skill Failure Layers, Sessions 共 12 个维度的卡片生硬罗列在一个垂直长列表中，用户无法一眼抓住核心业务指标与异常态势；
   - 顶部概览仅有5个孤立卡片，缺乏“健康度总览（Health Overview）”与黄金信号（Traffic, Latency, Errors, Saturation/Cost）的清晰比例与状态反馈。
2. **割裂的右侧下钻（Abrupt Dark-mode Theme Discontinuity）**：
   - 页面整体为温润浅色界面（`bg-[#faf6ef]` / `bg-white`），右侧会话下钻却突然切为纯黑大深色背景（`bg-stone-950 text-stone-100`），违反了 `design-taste-frontend` 的 Page Theme Lock 铁律，破坏视觉连续性并引起视觉疲劳；
   - 应该使用统一的高级石材材质与微妙投影系统（`bg-white/90`、柔和边框、暖色调背景、清晰的单色阶层）。
3. **缺少体系化的多维观测视角导航（Perspective Tabs / Segmented Views）**：
   - 用户想要分析“模型调用与成本分布”，或“扩展工具与能力生态（Capability/Skill）”，或“会话审计与下钻”，必须在超长滚动条中费力翻找；
   - 业内实践标准做法是：**全局黄金信号指示板 (Golden Signals Header)** + **四重视角导航条 (Perspective Tabs)**：
     - **视角 1：会话审计与下钻 (Sessions & Traces)**：双栏布局，左侧会话检索与指标列表，右侧深入的会话下钻、Turns 步骤、Trace 证据与 Timeline；
     - **视角 2：模型与供应商 (Models & Providers)**：按供应商与模型聚合的请求量、Token 吞吐（含缓存命中率）、首 Token 延迟对比；
     - **视角 3：工具与扩展能力 (Tools, Capabilities & Skills)**：整合原生工具、MCP/Capability Sources、Capabilities 详情与 Skill 调配层级；
     - **视角 4：治理与防御拦截 (Hooks & Guardrails)**：治理与安全 Hook 的阻断率、耗时统计与执行记录；
     - 同时保留 **全景平铺视图 (All Perspectives)**，满足习惯一览全量的用户。
4. **数据可读性与精细化交互体验（Data Legibility & Polish）**：
   - 会话搜索过滤功能：支持快速按标题/ID检索会话；
   - 状态徽章与健康指示：错误率、阻断状态、延迟层级具有温润的指示色（翡翠绿/琥珀黄/玫瑰红）；
   - 响应式自适应：在嵌入模式 (`props.embedded`) 和独立页面模式下，均有完美的视口利用率。

---

## 2. 契约与向前兼容规则 (Contract & 100% Backward Compatibility)
为了保障既有 665+ 项测试以及现有业务调用 100% 正常运行：
1. **DOM 契约与 data-testid 完全兼容**：
   - 现有的所有 `data-testid`（包括 `model-monitor-overview`、`model-monitor-refresh`、`model-monitor-providers`、`model-monitor-models`、`model-monitor-tools`、`model-monitor-sessions`、`model-monitor-session-*`、`model-monitor-drilldown-*`、`model-monitor-capability-*` 等）必须 100% 保留并在 DOM 中正确渲染；
   - 会话点击、切换、下钻刷新、Capability Sources 加载与检查逻辑严格保留，Tauri IPC 命令签名与调用时序零改动；
2. **无外部重型依赖（Zero Heavyweight Bloat）**：
   - 严禁借机引入大型图表库（ECharts/Chart.js/D3 等），采用轻量原生 SVG 条形柱图/迷你进度条/微指示器，加载开销 0ms，抗熵增。

---

## 3. 验收标准与门禁
1. 新增独立的单元与交互测试用例，覆盖：
   - 多重视角 Tab 切换正常且对应区域展现；
   - 会话搜索/过滤机制有效；
   - 现有测试用例（`tests/ModelMonitorPage.spec.ts`、`tests/TelemetryPage.spec.ts`）全绿通过；
2. 全量前端单测 `npm test` 保持 100% PASS。
