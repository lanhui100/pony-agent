# 重构阶段计划（当前状态总览）

## Phase 0：骨架阶段 ✅ 已完成

目标：
- 整理目录，保留 Hermes 参考区
- 建立 Tauri + Rust 基础骨架

## Phase 1：前端调试台 ✅ 已完成

目标：
- 接入 Vue 3 + Pinia + TypeScript + Tailwind CSS + shadcn-vue
- 三栏工作台：左侧导航、中间对话区、右侧可观测性面板
- 独立页面：Provider 配置页（ProviderConfigPage）、模型监控页（ModelMonitorPage）、设置面板（SettingsPanel）
- 附件中心（AttachmentCenterPanel）、Markdown 渲染、前端飞行记录仪

## Phase 2：单轮 Runtime ✅ 已完成

目标：
- run_turn() 与 start_turn_stream() 核心链路
- 多 provider 接入（OpenAI、Anthropic、DeepSeek 等）
- Provider 配置与凭证管理（SecretStore）
- 流式响应、turn loop（model → tool → model...）、cancellation
- 执行控制底座（stop_turn、load_execution_checkpoint）

## Phase 3：工具调用 ✅ 已完成

目标：
- ToolRouter 与 builtin_tools（17 个内部原语）
- 第一波产品级工具面（Plan/Read/Search/List/Edit/Write/Run/Ask）
- 第二波工具面（Glob/Grep/WebFetch/WebSearch/MCP Resource/ToolSearch）
- 工具权限模型与审批语义
- 工具观测读面与 telemetry

## Phase 4：会话与记忆 ✅ 已完成

目标：
- 多轮会话管理、摘要、本地持久化（SQLite + JSON 双后端）
- 分层 context/state subsystem（PA-018）
- LongTermMemory 独立边界与稳定事实
- 附件生命周期（active/missing_payload/reclaimable/expired）
- 会话控制审计面（history-control + run-control summary）
- 历史分支、checkout、restore、fork、switch branch

## Phase 5：Graph 编排 ✅ 已完成

目标：
- GraphRun 合同、状态机、runtime handoff 边界
- Graph orchestrator（GraphRunner、GraphRunStore）
- Graph stop/resume/checkpoint
- GraphPlanner（continue/wait_user 决策）
- HostControlPlane 统一控制面

## Phase 6：生命周期横切与能力接入 ✅ 已完成

目标：
- Agent hooks pipeline（observe/guard/transform/side_effect）
- 12 个 canonical hook boundary + run/memory/planner/capability/history-state hooks
- MCP capability bridge（统一 registry、tool/resource/prompt_template）
- Skills registry bridge（统一 skill 发现与执行）
- 缓存命中 telemetry（PA-029）
- Trace 面板 call model 可观测性（PA-030）

## Phase 7：基础设施加固 ✅ 已完成

目标：
- PA-044：core 独立为 Tauri-free workspace member（crates/pony-agent-core）
- PA-064：降低 session 切换与宿主读取压力（缓存优先）
- PA-065：拆分 runtime ownership 与锁分离
- PA-066：provider/tools 异步化（reqwest blocking → async）
- PA-067：收口 blocking 工作，建立统一 BlockingHelper
- PA-068：接入 per-session async turn task 模型
- PA-069 系列：Mutex RwLock、SQLite 写优化、锁序文档化

## Phase 8：高级能力 进行中

### 已完成
- 子代理系统（规划中）
- 图片查看（规划中）
- 权限请求（规划中）

### 进行中
- `PA-070` 统一 provider retry 与退避边界
- `PA-069-A~E` core 基础设施加固

### 规划中
- Workflow Mode（用户自定义流程编排）
- 子代理系统
- LSP 代码智能
- TodoWrite 任务管理
- Config 运行时配置管理
- 更丰富工具生态

## 缓存命中约束（横跨所有阶段）

缓存命中是 Pony Agent 架构设计的一等约束：
- Phase 2~3：避免反缓存结构，记录基础指标
- Phase 4：缓存友好结构正式进入设计（稳定层/半稳定层/易变层）
- Phase 5：graph run/planner/executor 会话边界显式化
- Phase 6+：子代理独立上下文、cache guard、回归测试

详见 ADR-0007（`docs/decisions/0007-cache-hit-as-first-class-product-metric.md`）。
