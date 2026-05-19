# 0012 当前 stream runtime、Hermes/Claude 对照与下一步架构路径

## 这次学习回答了什么

这一轮学习主要是在把“代码已经能跑”转成“我已经知道它现在处于哪一层、下一步往哪长”。

核心问题有五个：

1. 我们现在到底是不是已经有了真正的 stream
2. 当前前端和后端是如何通信的，是不是已经有 HTTP API
3. Hermes 和 Claude Code 在这一层的设计理念分别是什么
4. Pony Agent 未来是不是要二选一地走 Hermes 路线或 Claude 路线
5. 当前任务系统里的后续路径，是否已经包含这些设计方向

## 先说当前代码事实

当前项目已经不是“假流式”了，而是有一条真实的事件驱动流式主链。

### 1. 同步入口仍然存在

`run_turn()` 还在，它更像是：

- 最小单轮执行入口
- 同步整包返回的兼容形态
- 当前 runtime 领域对象的主承载点

### 2. 真正的流式入口已经长出来了

当前真正驱动工作台流式交互的，是并行新增出来的 `start_turn_stream()` 路径。

这条路径已经形成了最小事件模型：

- `turn:started`
- `turn:delta`
- `turn:trace`
- `turn:tool`
- `turn:completed`
- `turn:failed`

也就是说，Pony Agent 现在已经从“等整包返回”推进到了“增量回推 + 事件驱动 UI”阶段。

## 当前前端和后端是怎么通信的

这里有一个很关键的认识：

当前前端并不是通过 HTTP API 与后端通信。

它现在走的是：

- 前端 `invoke`
- Tauri command
- Rust 侧执行
- Tauri event
- 前端 `listen`

所以当前代码的真实关系是：

- Vue 工作台是桌面壳层
- Rust runtime 是本地 agent core
- Tauri 是当前这两层之间的桥

这也解释了为什么之前 `npm run dev` 会白屏。

因为普通浏览器里没有 Tauri runtime，前端如果直接调用 `@tauri-apps/api`，就会在浏览器预览模式下失效。现在我们已经补了浏览器环境兜底，所以开发预览和 Tauri 联调被清楚地区分开了。

## 如果未来要让 web 应用也使用当前 agent core，应该怎么做

答案不是“把前端改一下”。

正确方向是把当前结构继续拆成三层：

### 1. Core 层

这里放真正与 UI 无关的能力：

- runtime
- provider abstraction
- tool router
- session store
- query loop

### 2. Delivery Adapter 层

这里负责把同一个 core 暴露给不同宿主：

- Tauri adapter
- HTTP adapter
- SSE / WebSocket stream adapter
- CLI adapter

### 3. UI / Workbench 层

这里才是：

- 桌面工作台
- 未来 web 工作台
- 可能的调试面板

所以未来如果 web 应用要复用当前实现，不是重写 agent core，而是给 Rust core 新增一层 HTTP/SSE 交付适配器。

## Hermes 和 Claude Code 在这一层分别代表什么设计理念

这一轮最大的收获之一，就是把两者的“参考价值”分清了。

## Hermes 更值得借鉴的地方

Hermes 更像一个工程化的大 orchestrator。

它的参考价值主要在：

- 核心执行层与接入层之间有桥接意识
- callback / gateway 风格明显
- 更强调多端接入和工程联动
- 更像“如何把 agent 接到实际产品表面”

如果只看设计理念，Hermes 更像是在提醒我们：

不要让 UI、网关、运行时、展示层长成一团。

## Claude Code 更值得借鉴的地方

Claude Code 更像一个状态机化、生成器驱动的 agent loop。

它的参考价值主要在：

- query loop 是真正的核心
- stream、tool_use、tool_result 是一体化组织的
- 会话生命周期意识更强
- async generator / state machine 的味道更明显

如果只看设计理念，Claude Code 更像是在提醒我们：

agent 的本质不是一个“调用模型函数”，而是一个“可持续推进的回合循环”。

## Pony Agent 未来是不是二选一

不是。

这不是一条“选 Hermes”或者“选 Claude”的岔路，而是一条“按层借鉴”的演进路径。

最适合 Pony Agent 的组合方式是：

- 核心 runtime 更偏 Claude 式 query loop
- 接入层与桥接层更偏 Hermes 式 adapter / gateway 思路
- 存储、会话、工具系统按 Pony 自己的节奏渐进补齐

换句话说：

Pony Agent 当前更像教学版/最小版 runtime turn；
Hermes 更像多接入层的大型工程化 orchestrator；
Claude Code 更像状态机化、生成器驱动的完整 agent loop。

## 当前任务系统是否已经包含这些方向

是的，而且已经包含了。

至少从当前文档体系看，这条路径已经非常明确：

- 保留 `hermes/` 作为参考实现
- `run_turn()` 会继续向更完整的 query loop 演进
- 当前 stream、provider、runtime visibility 并不是终点，而是中间层

所以现在任务系统并不是停留在“做个能聊天的桌面 UI”，而是在为后续这些层做准备：

- provider trait
- tool router
- session persistence
- 独立 core
- HTTP/SSE adapter
- 更完整的 runtime loop

## 当前还没有的东西是什么

为了避免误判进度，也要明确现在还没做的部分：

1. 会话历史还没有真正持久化到数据库
2. 当前仍主要是前端内存态和运行时对象态
3. 还没有独立的 HTTP API / SSE 服务层
4. 还没有多工具、并发工具和更完整错误恢复
5. 还没有 Claude 那种更完整的 query loop 继续回合机制

## 那数据库、SQLite、Postgres、Redis 怎么看

从当前阶段看，最自然的演进顺序是：

1. 先抽 `SessionStore` 接口
2. 先落一个本地 SQLite 版本
3. 如果未来有服务化、多用户、部署场景，再补 Postgres
4. Redis 不是 Rust agent 的默认必需品，只有在队列、缓存、分布式协调这些场景下才会变得有价值

也就是说：

Rust 很强，不等于“天生不需要存储层”；
但 Rust agent 也绝不意味着“必须先上 Redis”。

## 当前学习进度

截至这一轮，已经清楚了这些事情：

1. 当前 Pony Agent 已经有真实 stream 主链，而不是只有同步整包返回
2. 当前工作台和 Rust core 的通信机制是 Tauri command/event，不是 HTTP API
3. `run_turn()` 是核心入口，`start_turn_stream()` 是当前流式工作链
4. 当前最小工具闭环已经成立，并且 live provider 已切到 OpenAI / Anthropic 原生 tools 协议主路径
5. Hermes 与 Claude Code 对 Pony 的启发不是互斥，而是分层借鉴
6. 当前任务系统已经把“独立 core、抽象层、后续 query loop 演进”放进主路径里了

## 下一步最自然的学习方向

如果继续沿这条线学习，最自然的顺序是：

1. 把 `runtime.rs`、`provider.rs`、前端 runtime store 三者之间的数据流再梳理一遍
2. 继续补工具层边界，包括更多工作区工具、工具错误态和多工具策略
3. 把当前事件契约收紧，补齐 `token 统计 / 首 token 延迟`
4. 设计 `SessionStore` 边界，区分“内存态会话”和“持久化会话”
5. 设计“同一 Rust core 同时服务 Tauri 与 HTTP/SSE”的交付接口
6. 再继续把 `run_turn()` 往更完整的 query loop 推进

## 一句总结

当前 Pony Agent 已经不再只是一个前端壳层 demo，而是进入了“最小流式 runtime 已成立，接下来要决定它如何长成独立 agent core”的阶段。
