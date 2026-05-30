# 0021 宿主层、控制面、runtime、graph 与 planner 应该怎么理解

## 问题

Pony Agent 当前为什么要拆出宿主层、控制面、runtime、graph、planner 这几层？它们分别是什么，和 Hermes agent、Claude Code 对应起来又该怎么理解？

## 简短结论

可以先记一句最重要的话：

- `runtime` 负责把“这一轮 turn 跑完”
- `graph` 负责决定“这一轮结束后，整趟任务接下来怎么办”
- `控制面` 负责把外部请求翻译成统一命令，并管理这些层的调用关系
- `planner` 不是一个单一概念，而是分成：
- `TurnPlanner`：判断这一轮怎么做
- `GraphPlanner`：判断这一轮结束后，整个 run 是继续、等待用户还是结束

在 Pony Agent 当前实现里，planner 主要是工程化判断，不是额外再起一轮 LLM planner。更准确地说，是“LLM 提建议，planner 做系统级裁决”。

## 系统化梳理

### 1. 先用一个最小心智模型理解

可以把整个系统想成一套任务推进流水线：

1. 用户或前端先把请求送进来
2. 控制面决定这是普通 turn，还是一个 graph run 命令
3. runtime 负责把单个 turn 执行完整
4. graph 消费这个已经稳定收口的 turn 结果
5. graph planner 决定接下来继续、等待用户、暂停还是结束

所以真正容易混淆的点，其实是 `runtime` 和 `graph`。

它们的区别不是“谁更高级”，而是“谁处理的时间尺度不同”：

- `runtime` 处理单个 turn 内的执行链
- `graph` 处理跨多个 turn 的任务生命周期

### 2. 宿主层和控制面不是一回事

宿主层更像“这个系统跑在哪里”，例如：

- Tauri 桌面端
- CLI
- 未来可能的 HTTP / SSE 宿主

控制面更像“统一中控台”，不管入口来自哪里，最后都收敛成同一套命令语义，例如：

- `run_turn`
- `start_graph_run`
- `continue_graph_run`
- `stop_graph_run`
- `resume_graph_run`
- inspection / health / checkpoint 读取

在 Pony Agent 里，这个角色已经比较明确，由 `src-tauri/src/agent/control_plane.rs` 里的 `HostControlPlane` 承担。

所以更准确的说法是：

- 宿主层负责入口形态
- 控制面负责统一命令语义

### 3. runtime 是什么

runtime 负责把一个 turn 跑完整。这里的“跑完整”包括：

- 组织 turn 上下文
- 做 provider 选择
- 执行模型请求
- 执行工具调用
- 处理 tool follow-up
- 更新 session
- 在需要时响应 cooperative cancel
- 输出稳定的 `TurnResult`

它关心的是：

- 这一轮有没有收完整
- 这一轮用了哪些工具
- 这一轮的流式事件怎么发
- 这一轮在什么地方被取消

它不关心的是：

- 任务整体还要不要再开下一轮
- 这个 goal 是否已经完成
- run 是否应该暂停等待用户

在 Pony Agent 里，这一层主要对应 `src-tauri/src/agent/runtime.rs` 里的 `AgentRuntime`。

### 4. graph 是什么

graph 不是“再做一次模型调用”，而是“管理 run 级状态机和任务生命周期”。

它更关心这些问题：

- 当前 run 处于什么阶段
- 上一轮 turn 是否已经稳定收口
- 这一轮结束后，是继续、等待用户、暂停还是结束
- stop / resume / checkpoint 应该怎么落
- run 级状态怎么持久化

在 Pony Agent 里，这一层主要对应：

- `src-tauri/src/agent/graph.rs` 中的 `GraphRun`
- `GraphRunner`
- `GraphRunStore`
- `GraphRunCheckpoint`

所以可以把 graph 理解成“任务流程管理器”，而不是“单轮执行器”。

### 5. 为什么要拆成 runtime 和 graph 两层

因为单个 turn 的执行问题，和整趟任务的编排问题，本质上不是一类问题。

如果把它们揉在一起，会马上遇到这些麻烦：

- 取消单轮执行和停止整个 run 会混在一起
- provider / tool follow-up 这种 turn 内细节会泄漏到高层编排
- checkpoint 不知道该建立在瞬时执行态上，还是稳定 handoff 上
- 后面很难解释“为什么这次自动继续了、下次没继续”

拆开之后，边界会清楚很多：

- runtime 只负责单 turn 执行完整
- graph 只消费稳定 turn 产物，不碰半成品 hop 中间态
- stop / resume / checkpoint 可以按 turn 级和 run 级分别建模

这也是 Pony Agent 当前设计里最重要的一条架构线。

### 6. 这里的 planner 是什么

planner 在当前 Pony Agent 里不是一个统一大脑，而是两个不同粒度的判断器。

#### `TurnPlanner`

这一层更接近“战术裁决”：

- 要不要在本地预判工具调用
- provider 给出的 tool call 是否要被本地方案替换
- 哪类高确定性请求可以先走本地工具

当前这层主要由 `src-tauri/src/agent/planner.rs` 中的 `LocalTurnPlanner` 实现。

#### `GraphPlanner`

这一层更接近“流程裁决”：

- 当前 turn 已经结束
- assistant 是否在明确向用户提问
- 当前 goal 是否支持 auto-continue
- 当前 run 是否已经接近自动推进上限
- 下一步到底是 `continue` 还是 `wait_user`

当前这层主要由 `src-tauri/src/agent/planner.rs` 中的 `DefaultGraphPlanner` 实现。

所以：

- `TurnPlanner` 决定“这一轮怎么做”
- `GraphPlanner` 决定“这一轮之后怎么走”

它们不是一个东西。

### 7. planner 当前更偏工程化判断，而不是 LLM planner

这是这次学习里非常关键的一点。

Pony Agent 当前的 planner 主要不是“再额外发一轮模型请求来做规划”，而是把稳定、可测、可审计的系统判断放在工程层。

这样做的设计意图是：

- 让工具前置命中更稳定
- 让 graph 的继续 / 等待用户 / 停止条件更稳定
- 让 stop / resume / checkpoint 建立在确定性规则上
- 避免把运行时稳定性完全交给 prompt 和模型漂移

更准确地说：

- LLM 负责理解、生成、提出候选动作
- planner 负责约束、裁决、编排和兜底

一句话总结就是：

“LLM 提建议，planner 做系统级裁决。”

### 8. Pony、Hermes、Claude Code 三者怎么对应

#### Pony Agent

Pony 是显式分层设计：

- `HostControlPlane` 是控制面
- `AgentRuntime` 是单 turn 执行器
- `GraphRun / GraphRunner / GraphRunStore` 是 run 级编排层
- `TurnPlanner / GraphPlanner` 是两个不同时间尺度的判断器

Pony 的特点不是功能最多，而是边界最清楚。

#### Hermes agent

Hermes 更像“功能很强的大单体 agent”。

它当然也有 runtime 和 graph 这两种语义，但很多职责是混在一起长出来的：

- `AIAgent.run_conversation()` 本身就承担了大量 runtime 语义
- CLI 的 `/goal` 机制承担了跨 turn 持续推进语义
- turn 结束后通过 `_maybe_continue_goal_after_turn()` 决定是否继续
- `process_loop()` 驱动整套 CLI 输入与执行主循环

所以 Hermes 更像：

- runtime 很强
- graph 能力是有的
- 但 graph 没有像 Pony 这样被抽成一个正式 run 状态机层

#### Claude Code

Claude Code 又不是 Pony 这种显式 graph run 风格。

它更像一个“会话操作系统 + 多 worker 编排平台”：

- `main.tsx` 是很强的宿主装配层
- `bootstrap/state.ts` 是非常强的 session / state 基座
- `tools.ts` 和 `commands.ts` 是平台式能力注册表
- `coordinatorMode.ts` 体现的是 coordinator / worker 编排

所以 Claude Code 的“graph 对应物”更像：

- coordinator 决定是否派 worker
- worker 完成后通过 `task-notification` 回流
- coordinator 再综合结果继续推进

它更偏“多代理协作编排”，而不是“单个 GraphRun 状态机推进”。

### 9. 当前关于 graph 的学习进度

截至这轮学习，关于 graph 这一层已经可以先稳定形成以下认识：

1. 宿主层和控制面不是一回事
2. runtime 只负责单 turn 执行，不负责 run 级任务编排
3. graph 负责 run 级状态机、stop / resume / checkpoint 和继续策略
4. `TurnPlanner` 和 `GraphPlanner` 不是同一个 planner
5. 当前 planner 主要是工程化裁决层，而不是独立 LLM planner
6. Pony 现在做的事情，本质上是在把 Hermes、Claude Code 中混合的语义显式拆层

这意味着 graph 这一层当前已经足够形成稳定心智模型，可以先暂停深入，后面在继续学习 memory、hooks、MCP、skills 时再回来看它如何与这些能力接边。

## 常见误区

- 误区 1：graph 就是单 turn 里的 while loop
- 实际上 turn 内 tool follow-up 仍属于 runtime；graph 处理的是 turn 之后的 run 级决策

- 误区 2：宿主层就是控制面
- 实际上宿主层解决“从哪里进来”，控制面解决“统一用什么命令语义往下调”

- 误区 3：planner 一定要是 LLM planner 才高级
- 实际上 planner 很多时候承担的是系统约束、停止条件、确定性裁决和兜底能力

- 误区 4：Hermes 和 Claude Code 没有 graph / runtime
- 它们有这些语义，只是没有像 Pony 这样显式分层命名和固化

## 后续值得继续学什么

- 为什么 `TurnPlanner` 更适合放在 runtime 侧，而 `GraphPlanner` 更适合放在 graph / 控制面侧
- context/state subsystem 进入之后，graph handoff 会不会发生变化
- hooks、skills、MCP 进入以后，哪些能力属于 runtime，哪些应该停留在 graph 或控制面

## 可延展内容选题

- 公众号：`为什么现代 agent 一定要把 runtime 和 graph 拆开`
- 公众号：`Hermes、Claude Code、Pony Agent 三种分层思路有什么本质差异`
- 知乎：`Graph planner 一定要用 LLM 吗，还是工程化规则更靠谱`
