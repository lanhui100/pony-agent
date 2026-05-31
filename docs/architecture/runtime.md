# Rust 运行时设计

## 第一阶段目标

先实现最小运行时闭环：

1. 接收输入
2. 调用模型
3. 返回结果
4. 在 UI 中展示状态变化

## 推荐模块

### `runtime`

负责：

- 启动一次 agent turn
- 协调 graph、provider、tool、session

### `graph`

负责：

- 描述执行状态和状态转移

第一版可以很简单：

- `Idle`
- `CallingModel`
- `CallingTool`
- `Done`
- `Failed`

### `provider`

负责：

- 对接模型
- 提供统一的 `generate` 能力

### `tools`

负责：

- 注册工具
- 执行工具
- 返回统一结果

### `session`

负责：

- 保存当前消息
- 管理会话元数据
- 维护 `sessionId -> session state` 的映射
- 为 runtime 提供 `snapshot / append_turn` 这种稳定边界
- 后续可从“内存 + JSON 文件”平滑升级到 SQLite / PostgreSQL

当前最小实现已经不是单个固定 `SessionState`，而是一个 `SessionStore`：

- 内存态负责当前进程内的快速读写
- 本地 `.pony-agent/sessions.json` 负责最小持久化
- `SessionBackend` 已预留成可替换后端接口，后续可接 SQLite / PostgreSQL
- `run_turn()` 与 `start_turn_stream()` 都只通过 session snapshot 读取上下文
- turn 完成后再统一 append 回 session store，而不是让前端长期承担真实会话态

### `config / secret`

负责：

- 管理 `providers.json` 里的非敏感 provider/model 配置
- 通过 Rust 侧 `SecretStore` 抽象管理 API Key 等敏感凭证
- 为 runtime 提供统一的 provider 选择与凭证解析边界

当前最小实现已经切到“配置与凭证分层”：

- `ProviderRegistryStore` 只持久化 provider、model、capability、selected state
- `api_key_value` 只作为本次保存请求的临时输入，不写回 `providers.json`
- 真实凭证优先写入 `SecretStore`
- runtime 解析优先级为 `runtime input > SecretStore > env fallback`
- `env` 现在只承担兼容旧配置与手工部署的兜底职责，不再是主存储

## 最佳实践

- 先抽象 trait，再落地 provider/tool 实现
- 返回结构化结果，不要到处传裸字符串
- 明确同步和异步边界
- 为调试留出事件或状态输出接口

## 当前边界

截至 2026-05-23，Pony Agent 的 runtime 已经有比较清晰的 core 边界：

- `runtime / context / provider / tools / session` 已能在非 Tauri 探针里直接复用同步 turn 与 provider 决策
- 当前真正仍绑定宿主的是“流式 turn 事件如何投递给前端”这层交付逻辑
- 这一层已完成第一刀抽离：core 通过 `TurnEventSink` 产出事件，Tauri 通过 `tauri_adapter.rs` 投递事件
- 因此下一步不再是“先去掉 `AppHandle`”，而是验证第二种 adapter 是否能复用同一套 core 事件模型

截至 2026-05-24，provider 配置边界也已进一步稳定：

- `config.rs` 已接入统一 `SecretStore`，不再把 API Key 当作普通配置落盘
- 默认平台后端优先使用系统密钥存储
- Windows：Credential Manager
- macOS：Keychain
- Linux：Secret Service / libsecret
- 若 Linux server / headless 环境缺少系统密钥服务，则降级到本地 `secrets.json`
- 这层降级是兼容性后端，不是新的主安全方案；`env` 仅保留读取兼容

## Adapter 策略

建议把职责拆成三层：

1. `core runtime`
   负责 turn 输入归一化、provider 选择、context 构造、tool 执行、session 回写、trace/tool activity 生成。
2. `desktop adapter`
   负责把 core 事件映射成当前 `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed` 这些 Tauri 事件名并投递给前端。
3. `future HTTP-SSE adapter`
   负责把同一套 core 事件映射成 SSE `event/data`，不重复实现 provider、session 或 tool 语义。

## 已完成的最小重构

- 已抽出统一的 turn event sink / callback 接口，让 `runtime` 不再直接依赖 `AppHandle`
- 已把 `turn_flow` 中的 `emit_*` 与 `stream_*chunks` 改成“core 产事件、adapter 投递事件”的两层
- 当前 Tauri event 名与前端消费契约保持不变，由 Tauri adapter 做映射
- 已新增 `sse_adapter.rs` 中的最小 SSE sink，把同一套 `TurnStreamEvent` 格式化成标准 `event/id/data` 帧
- 已新增 `sse_turn_probe.rs`，可在非 Tauri 环境下直接验证第二 adapter 对 core 事件流的消费

## 工具层收口

- 组合工具不再只靠隐式 nested result 表意，`workspace_batch / workspace_gather_context` 已开始显式输出 `ToolPlan`
- `ToolPlan` 当前包含：
- `kind`
- `summary`
- `parallel`
- `continueOnError`
- `steps[]`
- telemetry 已优先消费显式 `ToolPlan`，让“planned 子调用”和“completed 子调用”在 UI/trace 中有更稳定的解释边界

## 运行时稳健性

- 对 OpenAI 兼容 reasoning 模型，工具 follow-up 的流式调用现已具备“stream 失败时自动回退 sync follow-up，再继续按 delta 回放”的保守兜底
- 这让桌面端和未来 SSE 宿主都能共享同一条 core turn 流，而不是把 provider 兼容性问题泄漏到 adapter 层

## 下一步最小动作

- 继续观察真实 provider 下“大体积工具结果”的 follow-up 上限与压缩策略，避免把 provider 限制误判为 adapter 问题
- 继续确认哪些宿主能力应该走 adapter，哪些仍属于桌面端专有实现
- 继续把 `SecretStore` 与 provider registry 的边界稳定下来，为未来 HTTP / SSE / CLI / 桌面宿主复用做准备

## 2026-05-23 收口补充

- `ToolPlan` 已从临时的 `arguments.toolPlan` JSON 提升为 `ToolCall.plan: Option<ToolPlan>` 一等字段，`planner -> runtime -> telemetry` 现在都优先消费这条显式边界。
- `LocalTurnPlanner` 在显式多路径请求下会产出带 `plan` 的 `workspace_batch`，而 runtime 允许这类“显式计划型 preflight”在 native tool flow 下优先生效。
- OpenAI follow-up 现在对超大工具结果先做摘要 + 头尾裁剪；若 sync/stream follow-up 仍失败，则回退到本地整合响应，并显式暴露 `provider_mode=fallback` 与 `fallback_reason`。
- 本轮完成的验证：
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `npm run verify`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin direct_turn_probe -- multipath-context`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin direct_turn_probe -- large-result`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- adapter-multipath`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- adapter-large-result --raw`

## 2026-05-23 流式补充

- OpenAI-compatible provider 的工具 follow-up 流程现在会直接消费后端 SSE 响应，并在读取到每个 `data:` chunk 时立刻向 runtime 透传 `turn:delta`。
- 这意味着桌面端看到的 follow-up 文本与 reasoning 增量，已经不再是“整段响应收完后再本地切块回放”，而是真实 provider delta 驱动。
- 若 provider 流式请求失败，仍保持现有降级链路：`stream follow-up -> sync follow-up -> local fallback`，并显式暴露 `provider_source` / `fallback_reason`。
- 当前仍未完成真实 provider delta 透传的部分，是首轮 `plan_turn()` 直答路径；那部分仍属于后续 runtime loop / planning-stream 架构工作。

## 2026-05-24 turn 内多 hop 补充

- 本轮修复明确补在“单个 turn 内”的 provider/tool follow-up 小循环，而不是等待未来 graph 级任务 loop。
- `run_turn()` 与 `start_turn_stream()` 现在都会在同一 turn 内继续执行 `model -> tool -> model -> tool -> model`，直到拿到最终 assistant 文本，或触发 hop 上限保护。
- OpenAI tool follow-up 请求不再强制 `tool_choice="none"`；当模型在 follow-up 中继续请求下一次工具调用时，runtime 会继续留在当前 turn，而不是把过渡文本误发成 `turn:completed`。
- provider-native transcript 也随之扩展为多段 assistant/tool 往返，避免 session 恢复时把“半轮工具回合”误当作完整对话。

## 2026-05-24 执行控制底座现状

- `PA-010` 当前真正落地的是 runtime execution control substrate，不是 graph loop。
- Tauri 命令面已暴露：
- `start_turn_stream()`
- `stop_turn(turn_id)`
- `load_execution_checkpoint(turn_id | session_id)`
- `ExecutionControlRegistry` 维护 turn 级 `ExecutionCheckpoint`，包含：
- `status / phase`
- provider 元信息
- `completed_hops / max_hops / active_tool_name`
- trace / tool activity
- `stop_requested_at_ms`
- 取消模型是 cooperative cancel：runtime 在流式规划前、工具 hop 间隙和后续执行节点主动检查 `is_stop_requested(turn_id)`，命中后收口为 `cancelled` 终态。
- 当前取消结果会通过 `turn:cancelled` 事件发给宿主，前端据此结束提交态、写入“本轮已停止”消息并记录 turn trace。
- 前端在会话初始化或切换时也会消费 `load_execution_checkpoint(session_id)`，因此刷新或重新进入会话时，仍可恢复运行中的 turn、trace 与 tool activity。
- cancelled turn 现在会把用户消息与取消结果一并落回 session history，避免 reload 前后最后一轮不一致。
- 这一层已经足够给未来 graph 编排层消费“当前 turn 执行到哪了”，但还没有负责“是否继续下一轮 turn”的高层决策。

## turn loop 与 graph loop 的边界

- `turn` 内 loop 属于底层执行语义，目标是“把这一轮 assistant 回复收完整”。
- 它只处理单轮内的工具往返：`model -> tool -> model -> ... -> final answer`。
- 只要模型还在当前轮次里继续请求工具，就不应该提早发出 `turn:completed`。

- `graph` 级 loop 属于上层任务编排语义，目标是“围绕任务目标跨多个 turn 持续推进”。
- 它处理的是状态推进、阶段切换、是否开启下一轮 turn、是否等待用户输入、是否结束任务。
- `graph` loop 消费的应该是“已经完整收口的单个 turn 结果”，而不是半成品的 tool hop 中间态。

- 因此两者虽然都表现为 loop，但不是同一层：
- `turn loop` 解决“本轮还没答完”
- `graph loop` 解决“整个任务还没做完”

- 这也意味着当前的 turn 内工具 hop 上限、follow-up stream、provider-native transcript 完整性，都属于 `turn runtime` 的职责。
- 当前默认允许单个 turn 内最多 `1024` 次连续工具 hop；如需针对更强 agentic 模型继续放宽，可通过环境变量 `PONY_AGENT_MAX_TOOL_HOPS_PER_TURN` 覆盖，允许范围为 `1..=4096`。
- 这不是在把 `turn` 和 `run` 混为一谈，而是在承认现代 agentic 模型的“单轮收口”本身就可能很长；`run/graph` 不应该被拿来补偿一个尚未完成的 turn。
- 未来 graph 层不应拿“再开下一轮”去补偿“当前 turn 没收完整”的问题，否则会污染 session、trace、streaming 语义。

## 停止命令的分层

- 停止能力不能只做在前端，也不能只做在 graph。
- 它同样需要按层拆开：

### frontend / TUI / API

- 负责暴露“停止”入口，例如按钮、快捷键、HTTP endpoint、CLI 命令。
- 它们只负责表达用户意图，不负责定义真正的停止语义。

### adapter

- 负责把宿主入口翻译成统一 core 命令。
- 例如：
- `stop(scope=turn, id=turn_id)`
- `stop(scope=goal, id=goal_id)`

### runtime

- 负责“停止当前 turn / 当前流式执行”。
- 这层停止的是：
- 当前 provider 流
- 当前工具 hop
- 当前 turn 内 follow-up 小循环
- runtime 应持有 turn 级 cancellation / abort 能力，而不是仅靠 UI 忽略后续输出。

### graph

- 负责“停止整个任务编排，不再继续下一轮 turn”。
- 这层停止的是：
- 当前 goal
- 当前 graph run
- 后续是否还要继续发起下一轮 turn

- 因而：
- `Stop Turn` 属于 runtime
- `Stop Goal` / `Stop Run` 属于 graph
- 前端 / TUI / API 只是停止命令的入口，不应自己承载停止语义

## checkpoint 恢复的分层

- checkpoint 恢复同样不是单层能力。

### session / runtime checkpoint

- 恢复的是“执行到哪了”。
- 它关注：
- session history
- attachment asset catalog（`AttachmentAsset`）与 `session -> assetIds` 索引
- attachment lifecycle status（`active / missing_payload / reclaimable / expired`）与最小 cleanup 策略
- turn trace
- tool activity
- provider-native transcript
- 已完成的 turn 结果

- 这层适合：
- 应用重启后恢复对话
- 从上一次已完成的 turn 结果继续
- 恢复本地执行上下文

### graph checkpoint

- 恢复的是“整个任务推进到哪了”。
- 它关注：
- goal state
- 已完成 step
- 下一步计划
- budget 消耗
- blocked / paused 原因
- 子任务或分支状态

- 这层适合：
- 长任务中断后继续
- 自动编排流程暂停后恢复
- 跨多个 turn 继续完成同一个任务

### 分层约束

- runtime 不应直接承担高层任务树编排语义。
- graph 不应直接依赖 runtime 的瞬时内部变量或半成品 stream chunk。
- graph checkpoint 应建立在稳定的 runtime 产物之上，例如：
- `SessionSnapshot`
- `TurnResult`
- `ExecutionCheckpoint`

- 然后由 graph 进一步组合成：
- `GoalCheckpoint`
- `GraphRunCheckpoint`

## 对 PA-010 的约束

- 因为有上述分层，`PA-010` 不应实现“goal-driven graph loop”本身。
- `PA-010` 更准确的范围应是 runtime execution control substrate：
- turn / run 的停止原语
- budget / stop condition contract
- execution state / checkpoint substrate
- 供未来 graph loop 消费的稳定运行控制边界
- 截至 `2026-05-24`，已完成的是：
- turn 级 stop 命令入口
- turn 级 checkpoint 读取入口
- cooperative cancel 检查
- `cancelled` 事件与终态落账
- 截至 `2026-05-24`，尚未完成的是：
- goal / run 级 stop
- graph loop
- graph checkpoint
- 完整的 budget_exhausted / timeout / consecutive_error 停止条件矩阵

## 下一阶段 graph/runtime 拆分（2026-05-24）

- 在 `PA-010` 收口之后，graph/runtime 主线按三张卡推进：
- `PA-012`：先定义 `GraphRun` contract、graph 状态机与 runtime handoff 边界
- `PA-013`：再实现最小 graph orchestrator，只消费完整 turn 结果，不碰半成品 hop 中间态
- `PA-014`：最后补 run/goal 级 stop、graph checkpoint、resume 与 stop-condition 矩阵
- 这条主线之外，可并行推进：
- `PA-015`：宿主控制面，把 Tauri、HTTP/SSE、CLI 收敛到统一命令与 inspection 面
- 范围约束：
- 不把 graph loop 回填进 `PA-010`
- 不让 adapter 承担 graph 调度逻辑
- 不让 graph checkpoint 依赖 runtime 瞬时内部变量

## 2026-05-31 更新
- `PA-013 / PA-014 / PA-017` 已完成并通过回归，近线 graph/runtime/attachment 拆分已经收口。
- runtime 与 graph 的职责边界保持不变：runtime 继续只负责单 turn 执行、tool follow-up、stream 与 cooperative cancel；graph 负责 run 级 orchestrator、stop/resume/checkpoint 与 run 状态持久化。
- graph checkpoint 现在明确建立在稳定 handoff 之上，而不是 runtime 内部瞬时变量；当前稳定输入已经收口为 `TurnResult + RetrievedContextState -> GraphTurnHandoff`。
- graph 侧新增的稳定产物包括：`GraphRunStopReason`、`GraphRunCheckpoint`、`active_turn_id`、`last_completed_turn_id`、`last_handoff`、`resume_count`。
- 附件层新增的稳定产物包括：`AttachmentLifecycleStatus`、`AttachmentAssetQuery`、`AttachmentCleanupRequest` 与显式 cleanup；生命周期状态现在至少覆盖 `active / missing_payload / reclaimable / expired`。
- `PA-018` 已完成，runtime 不再默认依赖原始 `SessionSnapshot` 来构建请求、planner preflight 或 graph handoff。
- 下一阶段与 runtime 最相关的主线转为：
  - `PA-025`：`RetrievedContextState -> prompt/request` 映射与 cache-friendly prompt 边界
  - `PA-024`：retrieval observability / trace 展示语义
## 2026-05-25 graph planner 补充
- `PA-019` 已建立最小 graph planner / policy：`GraphPlanner / GraphPlanningContext / DefaultGraphPlanner` 不进入 turn runtime，而是由 graph run 在稳定 handoff 之后消费。
- 当前 graph policy 仍保持保守：assistant 明确在向用户提问时输出 `wait_user`；goal 带有多轮推进信号、且本轮已稳定收口时输出 `continue`；其余情况默认 `wait_user`。
- `continue` 当前只把 run 保持在可继续状态，不会自动递归再跑下一轮 turn；这继续保持了 “turn loop 属于 runtime、graph loop 属于编排层” 的边界。
