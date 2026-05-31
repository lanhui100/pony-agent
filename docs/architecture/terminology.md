# 架构术语表

## 目的

这份术语表用于固定 Pony Agent 当前阶段最容易混淆的一组概念：

- `context`
- `state`
- `memory`
- `retrieval`
- `checkpoint`
- `transcript`

目标不是补充更多抽象名词，而是减少讨论时的语义漂移，让架构、任务系统和代码实现对同一批词保持一致理解。

## 总原则

1. 不再把一切“会被保留的信息”都统称为 `memory`
2. `memory` 只保留给真正具有长期沉淀语义的部分
3. 当前 turn、当前 session、当前 run 的大部分信息优先归入 `context` 或 `state`
4. 上层模块不应直接翻历史消息猜状态，而应通过稳定的 `retrieval boundary` 读取结构化结果

## 核心术语

### `Context`

`Context` 指某一层在“当前这次消费”中需要看到的上下文材料。

它的特点是：

- 面向消费者
- 常常是临时的
- 不要求长期保存
- 可以由多种底层信息组装出来

典型例子：

- 当前 turn 里的工具结果
- 当前用户这轮上传的图片
- 当前会话最近几轮的稳定约束
- 为模型构造 prompt 时需要注入的结构化上下文

### `State`

`State` 指系统当前运行到哪、后续怎么继续、怎么恢复的结构化状态。

它的特点是：

- 面向系统控制流
- 可持久化
- 可检查
- 可恢复

典型例子：

- `GraphRun.phase`
- `resume_count`
- `active_turn_id`
- `ExecutionCheckpoint`
- 当前 run 已完成了哪些 step

### `Memory`

`Memory` 指跨 turn、跨 session、甚至跨 run 仍值得沉淀和复用的知识性信息。

它的特点是：

- 生命周期更长
- 写入更谨慎
- 不等于原始 history
- 更接近被整理后的稳定知识

典型例子：

- 用户长期偏好
- 项目长期约定
- 稳定摘要
- 后续可检索的长期知识条目

### `Retrieval`

`Retrieval` 指上层模块如何读取 `context / state / memory` 的稳定边界。

它的重点不是“底层怎么存”，而是“上层怎么拿”。

要求是：

- 返回稳定结构
- 隐藏底层存储细节
- 不要求调用者直接读 `history / transcript / checkpoint` 原始材料

### `Checkpoint`

`Checkpoint` 指为恢复执行或恢复任务推进而保存的状态快照。

它是 `state` 的一种特殊形式，不是 `memory`。

可再细分为：

- `ExecutionCheckpoint`：turn/runtime 级
- `GraphRunCheckpoint`：run/graph 级

### `Transcript`

`Transcript` 指 provider、assistant、tool 往返过程中产生的原始过程记录。

它的特点是：

- 更接近原始执行轨迹
- 可用于调试、审计、回放
- 不应直接等同于长期记忆

## 当前推荐分层

### `TurnContext`

定义：
当前 turn 为完成本轮推理与工具收口所需的上下文。

常见来源：

- 用户本轮输入
- 本轮图片
- 本轮工具结果
- 本轮临时摘要

不应默认变成：

- 长期记忆
- graph run 状态

### `SessionContext`

定义：
当前会话中持续有效、但不一定跨会话保留的上下文。

常见来源：

- 最近几轮稳定约束
- 当前会话摘要
- 最近引用的文件
- 会话级附件引用关系

### `RunState`

定义：
某条 graph run 的执行状态与推进位置。

常见来源：

- `GraphRun`
- `GraphRunCheckpoint`
- planner 决策输入
- 已完成 step 与下一步状态

注意：
`RunState` 不是 `RunMemory`。这里更准确的语义是“状态”，不是“记忆”。

### `LongTermMemory`

定义：
跨会话、跨 run 仍值得保留的稳定知识。

注意：
这是当前最适合继续使用 `memory` 一词的地方。

## 当前代码映射

- `TurnContext`
  主要映射到 [runtime.rs](C:\Users\HUAWEI\Documents\pony-agent\src-tauri\src\agent\runtime.rs) 中围绕 `TurnInput`、工具 follow-up、图片输入与上下文构造的部分

- `SessionContext`
  主要映射到 [session.rs](C:\Users\HUAWEI\Documents\pony-agent\src-tauri\src\agent\session.rs) 中的 `SessionStore`、`SessionSnapshot`、history、session summary 与附件引用

- `RunState`
  主要映射到 [graph.rs](C:\Users\HUAWEI\Documents\pony-agent\src-tauri\src\agent\graph.rs) 中的 `GraphRun`、`GraphRunCheckpoint`、phase、resume_count、handoff

- `ExecutionCheckpoint`
  主要映射到 [runtime.md](C:\Users\HUAWEI\Documents\pony-agent\docs\architecture\runtime.md) 中描述的 turn 级执行快照

- `Transcript`
  主要映射到 provider/tool 往返过程中的原始执行记录与相关调试材料

## 命名约束

后续命名时优先遵循下面这些规则：

1. 如果某结构主要描述“当前给谁看什么”，优先用 `Context`
2. 如果某结构主要描述“系统当前到哪了”，优先用 `State`
3. 如果某结构主要描述“跨会话长期保留的知识”，再用 `Memory`
4. 如果某能力主要描述“上层怎么拿数据”，优先用 `Retrieval`
5. `Checkpoint` 只用于恢复相关快照，不用于泛化表示任何持久化信息
6. `Transcript` 只用于过程记录，不用于表示摘要或长期知识

## 反模式

以下说法默认应避免：

- “把 run memory 读出来继续执行”
  更准确说法：读取 `RunState` 后继续执行

- “把 checkpoint 当 memory”
  更准确说法：checkpoint 是可恢复状态，不是记忆

- “把所有 history 都当 memory”
  更准确说法：history 是原始材料，可能被加工成 context、state 输入或 long-term memory

- “planner 直接从聊天历史里猜任务状态”
  更准确说法：planner 应优先消费稳定的 retrieval 结果

## 与 PA-018 的关系

`PA-018` 完成后，应该把它理解为“retrieval boundary 收口任务”，而不是“长期记忆产品任务”：

- 建立 `TurnContext / SessionContext / RunState / LongTermMemory` 的分层边界
- 建立它们的统一 `retrieval boundary`
- 让 runtime、graph、planner 与宿主默认查询面优先消费结构化 retrieval 结果

这也意味着：

- `LongTermMemory` 是其中一层，不等于整个子系统
- retrieval 观测与 trace 展示语义不再留在 `PA-018`，而是转交 `PA-024`
- `RetrievedContextState -> prompt/request` 映射、`Build Context` 解释力与 cache-friendly prompt 边界不再留在 `PA-018`，而是转交 `PA-025`

对应任务卡见：

- [PA-018-build-context-state-subsystem-and-retrieval-boundary.md](C:\Users\HUAWEI\Documents\pony-agent\management\task-system\03_TASKS\PA-018-build-context-state-subsystem-and-retrieval-boundary.md)
