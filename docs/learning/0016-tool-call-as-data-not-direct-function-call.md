# 0016 为什么工具调用要先变成 ToolCall / ToolResult，而不是直接调函数

## 这次学习回答了什么

这一轮学习不再纠结“某个工具能不能跑”，而是回答一个更底层的问题：

- 为什么 Pony Agent 里的工具调用不能理解成“模型直接调函数”
- 为什么一定要先抽象成 `ToolCall`
- 为什么执行完还要再抽象成 `ToolResult`

## 先说结论

Pony 里的工具调用，本质上不是“谁直接去执行一个函数”，而是：

1. 先把工具调用表达成一份结构化数据
2. runtime 再去执行这份数据
3. 再把执行结果重新包装成结构化结果
4. 然后把结果放回当前回合上下文

也就是说，工具调用在 Pony 里分成两层：

- 上层：决定调什么工具，产出 `ToolCall`
- 下层：执行 `ToolCall`，返回 `ToolResult`

## 为什么不能让模型直接调函数

如果让模型或 runtime 直接绑到某个具体函数，会有几个问题：

### 1. 耦合过重

模型、runtime、工具实现会直接绑死在一起。

这样后面一旦：

- 工具实现换成 shell
- 换成 MCP
- 换成 HTTP service
- 换成远端 worker

上层流程就会被迫一起改。

### 2. 不可观测

如果没有 `ToolCall / ToolResult` 这一层结构化对象，就很难稳定记录：

- 本轮到底调了哪个工具
- 参数是什么
- 成功还是失败
- 返回了什么

而这些恰恰是 agent runtime 最重要的调试信息。

### 3. 无法演进到多工具和并发工具

未来如果要做：

- 多工具链式调用
- 并发工具
- 工具权限控制
- 工具失败重试

都需要先把“工具调用”变成可操作的数据对象，而不是直接埋死在函数跳转里。

## 当前 Pony 的设计理念

当前 Pony 的工具链路可以概括成：

- decision 阶段先决定要不要调工具
- 如果要调，就生成 `ToolCall`
- `ToolRouter` 根据 `ToolCall` 找到对应工具实现
- 工具执行后生成 `ToolResult`
- `ToolResult` 回到当前回合上下文
- provider 再基于 `ToolResult` 继续生成最终回答

所以真正推进回合的，不是“直接函数调用”，而是：

- `ToolCall`
- `ToolResult`

这两个结构化对象。

## 这对架构意味着什么

这一层设计的价值，在于它把“决定”“执行”“生成回答”三件事分开了：

### 1. planner / decision 层

负责判断：

- 要不要调工具
- 调哪个工具
- 参数大致是什么

### 2. tool execution 层

负责真正执行：

- 本地函数
- 或未来的 shell / MCP / HTTP service

### 3. provider generation 层

负责基于工具结果继续组织自然语言回答。

这就是为什么说，Pony 不是“模型直接调函数”，而是“runtime 驱动的一条结构化工具链”。

## 当前学习进度

截至这一轮，可以明确地说：

1. 工具调用已经不是黑盒
2. `ToolCall / ToolResult / ToolRouter` 已经成为 Pony 当前工具层的最小骨架
3. runtime、provider、tool execution 三层边界已经比早期清楚得多
4. 后面的多工具、并发工具、工具权限控制，都会建立在这层抽象上

## 和 Hermes / Claude 的对比理解

这一层设计并不是 Pony 自己“凭空发明”的，它和成熟 agent runtime 的思路是一致的。

两边虽然实现风格不同，但共同点很明确：

1. 都不会把“模型输出”直接等同于“本地函数调用”
2. 都会先把工具调用表达成结构化的中间对象
3. 都会经过 runtime 的调度或执行层
4. 都会把工具结果重新写回当前回合上下文，再继续推理

也就是说，不管是 Hermes 还是 Claude，工具调用本质上都是“agent loop 里的一个运行时步骤”，而不是“模型越过 runtime 直接碰实现”。

### Hermes 的设计取向

Hermes 更像一个平台型 runtime。

它的重点不是只解决“这一轮怎么调工具”，而是把整个 agent 运行时统一收拢起来，包括：

- prompt 组装
- provider 适配
- tool schema 暴露
- central registry / dispatch
- approval 和 guardrail
- 并发执行
- session persistence
- compression 和 fallback

所以 Hermes 的工具层设计理念是：

- 工具是一等运行时能力
- 工具不只是“能调用”，还必须“可治理”
- tool call 要天然接入审批、并发、持久化、可观测性这些机制

从这个角度看，Hermes 更像“大 orchestrator + registry/dispatch runtime”。

### Claude 的设计取向

Claude 的思路更像“query loop + tool execution service”。

它更强调：

- 工具调用是消息流的一部分
- 工具执行是 query state machine 的一步
- 工具编排、权限、校验、hook、telemetry 要拆成独立层

所以 Claude 的重点不是把所有能力塞进一个巨大 agent 类里，而是把一轮 turn 内部的职责拆清楚：

- query loop 负责回合推进
- orchestration 负责串行 / 并发编排
- execution 负责真正执行单个 tool use
- tool result 再回到消息上下文

从这个角度看，Claude 更像“状态机化的 runtime core”。

### 这对 Pony 的启发

Pony 当前阶段更接近 Claude 这一侧的主干思路：

- 先把 `run_turn`
- `decision / planner`
- `ToolCall / ToolRouter / ToolResult`
- history carry

这些最小运行时骨架打通。

这是因为我们现在还处在 agent core 的早期阶段，优先级最高的是把“单轮怎么稳定跑通”这件事做好。

但往后继续演进时，Hermes 的那部分理念也会逐步进入 Pony，例如：

- 更完整的 tool registry
- 工具可用性过滤
- approval / safety guard
- session store / persistence
- 并发工具调度
- 更强的 telemetry 和 diagnostics

所以这不是 Hermes 和 Claude 的二选一，而是两个观察角度：

- Claude 更适合指导我们先搭好 runtime core
- Hermes 更适合指导我们后面把 runtime 做成可治理的平台

## 这一轮新增的理解

截至这一轮，关于 tool call 可以再补充一个更稳定的认识：

1. `ToolCall / ToolResult` 不是语法细节，而是 runtime 的边界对象
2. Hermes 和 Claude 都在用不同方式证明这层边界是必要的
3. Pony 当前先采纳 Claude 式的最小运行时主干是合理的
4. 后面再逐步吸收 Hermes 式的平台治理能力，会比一开始就做“大而全”更稳

## 下一步最自然的方向

工具调用机制本身已经讲清楚了，接下来更自然的重构重点不再是“工具如何调用”，而是：

1. 如何把当前临时 history 提升成更稳定的 session/runtime state
2. 如何把上下文组织得既利于 agent 理解，也利于 provider cache 命中
3. 如何继续扩展工作区工具，而不破坏当前工具层边界

## 一句总结

Pony 里的工具调用不是“直接调函数”，而是“先把调用表达成数据，再由 runtime 执行数据并返回结构化结果”。
