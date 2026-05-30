# 架构总览

## 目标

Pony Agent 的目标是构建一个可观察、可扩展的桌面智能体系统，而不是简单的聊天壳。

在阅读本页前，若对 `context / state / memory / retrieval / checkpoint / transcript` 的含义还不稳定，先看 [架构术语表](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/terminology.md)。

## 目标分层图（规划态）

1. 宿主层：`Tauri / CLI / HTTP-SSE / TUI`
   职责：承载具体入口形态、窗口或进程生命周期、用户交互面。
2. 宿主控制面：统一命令与状态查询
   职责：把不同宿主入口翻译成同一组 `turn / run / session / telemetry / attachment / checkpoint` 命令与 inspection 读取面。
3. Graph 编排层：`goal / run / planner / stop / resume / checkpoint`
   职责：围绕任务目标做跨 turn 编排、停止/恢复、budget 与 checkpoint 决策。
4. Runtime 执行层：单 turn loop / tool follow-up / stream / cancel
   职责：把单个 turn 真正执行完，处理 `model -> tool -> model -> ... -> final answer`、流式 delta 和 turn 级取消。
5. 能力接入层：`tools / MCP / skills bridge`
   职责：把外部能力纳入统一 capability registry，提供可被 runtime / graph 消费的稳定能力边界。
6. 状态、上下文与记忆层：`session / attachments / context / state / memory / transcript / checkpoints`
   职责：承载会话、附件、当前上下文、运行状态、长期记忆、provider transcript 与各层 checkpoint 的稳定状态面。
7. 基础设施层：`provider / secret / telemetry / storage`
   职责：提供模型协议、密钥存储、遥测、文件/数据库存储等底座能力。

这张图描述的是最终目标分层，不表示所有层都已经完成。

## 当前已落地切片

### 1. Frontend

职责：

- 展示聊天与运行状态
- 展示工具调用和 Graph 轨迹
- 作为调试台承载交互

建议栈：

- Vue 3
- TypeScript
- Pinia

### 2. Tauri Shell

职责：

- 承载桌面窗口
- 提供前后端命令桥接
- 管理桌面应用生命周期

截至 `2026-05-24` 的宿主边界约束：

- Tauri 入口只保留参数翻译、状态注入与事件投递，不承载独立调度逻辑
- `turn / session / checkpoint` 命令先进入统一的 `host control plane v1`，再下探 runtime / execution control
- 后续 HTTP/SSE/CLI 需要复用同一套 core 命令和 inspection 读取面，而不是各自复制入口语义

### 3. Rust Runtime

职责：

- 接收用户输入
- 组织单轮执行
- 调用模型
- 决定是否调用工具
- 管理执行状态

当前分层约束：

- `turn` 内 loop 属于 runtime 的底层执行语义，用来把单个 turn 收完整。
- 它处理的是单轮内的 `model -> tool -> model -> ... -> final answer`。
- 只要当前轮次里模型还在继续请求工具，就不应该提早结束这一轮。
- 截至 `2026-05-24`，这里已落地的是执行控制底座，而不是 graph loop：`start_turn_stream()`、`stop_turn()`、`load_execution_checkpoint()`、协作式取消检查和 `turn:cancelled` 事件都属于 runtime 边界。

- 未来 `graph` 级 loop 属于更高层的任务编排语义。
- 它处理的是跨多个 turn 的目标推进、状态迁移、是否开启下一轮、是否等待用户输入。
- `graph` 层消费的应该是“已经完整完成的单个 turn 结果”，而不是 tool hop 中间态。
- 当前代码库里还没有 goal-driven graph loop、跨 turn budget 调度或 graph checkpoint 恢复。

停止与恢复也按这一边界拆层：

- `runtime` 负责：
- stop current turn
- turn 级 cancellation
- execution checkpoint substrate
- `graph` 负责：
- stop current goal / run
- graph 级恢复
- 跨 turn 的任务推进与继续条件

- `frontend / TUI / API` 只是命令入口
- `adapter` 负责把入口翻译成统一 core 命令

这也意味着：

- 宿主层与宿主控制面不是一回事；前者负责入口形态，后者负责统一命令语义。
- `turn` 内 loop 不属于 graph 编排层，而属于 runtime 执行层。
- 未来 `stop goal / resume graph / graph checkpoint` 也不应回塞到 runtime。

### 4. Provider Layer

职责：

- 封装不同模型接口
- 对外暴露统一调用协议

### 5. Tool Layer

职责：

- 管理工具定义
- 路由工具调用
- 返回结构化工具结果

### 6. Session / Context / State Layer

职责：

- 管理会话上下文
- 承载运行时状态与附件引用边界
- 为未来长期记忆沉淀提供稳定入口

截至 `2026-05-24` 的多模态边界：

- 已落地：用户消息附件现在拆为 `AttachmentReference` 与独立 `AttachmentAsset` catalog；`SessionStore` 同时维护 `session -> assetIds` 跨会话索引；最近一轮图片仍按引用语义重新召回；前端消息可展示 `displayMessage` 与历史附件 chip。
- 已补最小生命周期：附件资产现在带 `active / missing_payload / reclaimable / expired` 状态，可在 `SessionStore` 边界按 `session / mime type / name / time` 做最小检索，并由显式 cleanup 只回收未被历史引用的资产。
- 已收紧：recent-image recall 只会回附“最近一轮 user turn 自己带的图片”，不再跨多轮模糊重发旧图；前端 replay history 也只会回传已持久化的 canonical 附件。
- 本阶段未落地：TTL、清理管理台、完整附件中心 UI、完整搜索体验。

### 7. Config / Secret Infrastructure

职责：

- 管理 provider registry、模型能力声明与用户策略
- 将敏感凭证与普通配置分层
- 用统一 `SecretStore` 抽象屏蔽平台差异

当前边界：

- `providers.json` 保存非敏感配置
- `SecretStore` 保存 API Key
- 平台后端优先走系统密钥存储，`env` 仅作兼容 fallback

## 下一阶段拆分（2026-05-24）

- `PA-010 / PA-011` 已完成本轮收口：前者只覆盖 runtime execution control substrate，后者只覆盖最小多模态 session memory。
- 下一阶段 graph/runtime 主线拆为：
- `PA-012`：定义 `GraphRun` contract、graph 状态机与 runtime handoff 边界
- `PA-013`：实现最小 graph orchestrator，围绕完整 turn 结果推进多 turn run
- `PA-014`：补 run/goal 级 stop、graph checkpoint、resume 与 stop-condition 矩阵
- 并行宿主控制面拆为：
- `PA-015`：统一 Tauri、HTTP/SSE、CLI 的 control plane 与 inspection 读取面
- 附件中心线拆为：
- `PA-016`：建立附件资产目录与跨会话索引
- `PA-017`：补附件生命周期、检索与最小管理面
- 后续能力层拆为：
- `PA-018`：建立分层 context/state subsystem 与 retrieval boundary
- `PA-019`：建立 graph planner 与计划决策策略
- `PA-020`：建立 MCP capability bridge
- `PA-021`：建立 skills registry 与 bridge
- `PA-022`：建立 lifecycle hooks pipeline
- 与目标分层图的对应关系：
- `PA-015` 对应宿主控制面
- `PA-012 ~ PA-014` 对应 graph 编排层
- `PA-010` 对应 runtime 执行层底座
- `PA-016 ~ PA-018` 对应状态、上下文与记忆层中的附件与 context/state 主线
- `PA-020 / PA-021` 对应能力接入层
- `PA-022` 对应横切生命周期机制

## 2026-05-25 更新
- `PA-013` 已把 `GraphDecision` 接成最小 graph orchestrator：`GraphRunStore / GraphRunner / start_graph_run / continue_graph_run / inspect(include_run)` 已落地。
- `PA-014` 已把 graph stop/resume/checkpoint 提升到 graph 层：当前有 `GraphRunStopReason`、`GraphRunCheckpoint`、run 持久化 store、`stop_graph_run / resume_graph_run / load_graph_run_checkpoint`。
- `PA-017` 已把附件资产从“只能 recent recall”推进到“可查询、可清理、可审计”的最小产品级状态：当前具备 `active / missing_payload / reclaimable / expired` 生命周期状态、最小查询面与显式 cleanup。
- 当前新的近线任务已经切到 `PA-018` context/state subsystem 与 `PA-019` graph planner；`PA-020 ~ PA-022` 仍保持在后续能力层。
- 截至 `2026-05-28`，`PA-018` 已经开始第一阶段实现：runtime 已新增 `RetrievedContextState / ContextStateRetriever` 合约，`build_request()` 与 planner preflight 已开始消费结构化 retrieval 结果。详见 [Context/State 子系统 V1](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/context-state-subsystem.md)。
- 推荐顺序：
- 先做 `PA-018 / PA-019`
- 再做 `PA-020 / PA-021`
- 最后做 `PA-022`
- 范围约束：不要把 graph loop、goal/run stop、附件中心生命周期继续回填到 `PA-010 / PA-011`。

## 未来能力层定位

- `PA-018` 里的总称不再建议继续使用 `memory subsystem`。更准确的语义是：它要建立 `TurnContext / SessionContext / RunState / LongTermMemory` 的统一 retrieval boundary。
- 这里只有 `LongTermMemory` 真正属于“记忆”；`TurnContext`、`SessionContext` 与 `RunState` 更接近上下文和状态，而不是长期记忆。
- graph planner 属于编排层内部能力，不应重新塞回 turn runtime。
- MCP 属于能力接入层，不是调度层。
- skills 属于能力封装层，不是执行底座层。
- hooks 属于横切机制层，应该建立在前述主线都稳定之后。

## 与 Hermes 的关系

- `hermes/` 是学习对象，不是主开发区
- Pony Agent 的目标是提炼 Hermes 的有效经验，并用更清晰的 Rust 分层重建
- 对 Hermes 的借鉴以“思想迁移”为主，不以“代码搬运”为主
## 2026-05-25 补充
- `PA-019` 已把 graph planner 的边界正式固化：`GraphPlanner / GraphPlanningContext / DefaultGraphPlanner` 位于 graph 层，只消费稳定 `GraphRun + GraphTurnHandoff`，输出可审计的 `continue / wait_user` 最小 policy。
- 这意味着 “是否继续下一轮” 已从 runtime turn preflight 中抽离出来；runtime 仍只负责单 turn 执行，graph 才负责 run 级决策。
- 前端新增的 `Graph Run` 控制面与附件中心 UI 只是宿主消费层，对应这份分层图里的宿主层 / 宿主控制面，不承载 graph/runtime 核心语义。
