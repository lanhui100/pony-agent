# 0013 为什么要从最小 tool 闭环切到 OpenAI/Anthropic 原生 tools 协议

## 这次学习回答了什么

这一轮学习重点不是“tool 能不能跑”，而是“tool 跑通之后，下一步应该把协议停在哪一层”。

核心问题有三个：

1. 为什么一开始先做最小 tool 闭环
2. 为什么在最小闭环成立后，又要继续切到 OpenAI / Anthropic 原生 tools 协议
3. Pony 内部 runtime 抽象和外部 provider 协议之间，边界应该怎么划

## 先说结论

最小 tool 闭环和原生 tools 协议，不是互相否定的两条路，而是连续的两个阶段。

可以这样理解：

- 第一阶段：先验证 Pony 自己的 agent loop 骨架是否成立
- 第二阶段：再把这个骨架接到真实 provider 的原生 tools 协议上

也就是说，前一阶段解决的是“Pony 内部结构问题”，后一阶段解决的是“外部协议适配问题”。

## 为什么一开始要先做最小 tool 闭环

因为当时最急需验证的，不是 OpenAI 或 Anthropic 的字段细节，而是这条链本身有没有成立：

1. 模型先做决策
2. runtime 识别要不要调工具
3. Rust 执行工具
4. 工具结果回写到回合
5. 模型基于工具结果继续回答

如果这一层都还没有成立，就直接去接双 provider 原生 tools，会把问题混在一起：

- 是 query loop 设计不对
- 还是 provider 协议接入不对
- 还是前端事件流承不住

所以最小闭环的价值，在于先把“内部 agent loop”验证出来。

## 为什么最小闭环成立后，不能长期停在中间协议

因为那样会让 Pony 长期停留在“教学过渡层”，而不是进入真实的 provider runtime。

如果长期依赖中间文本约定，会带来几个问题：

### 1. provider 的真实行为被屏蔽了

模型到底是通过什么机制触发工具：

- OpenAI 的 `tool_calls`
- Anthropic 的 `tool_use`

如果都被统一改写成文本约定，很多真实运行时问题就看不到了。

### 2. 后续 stream 与 tool event 的真实性会不足

未来真正产品化时，最重要的是知道：

- 工具调用何时出现
- tool call id 是什么
- tool result 如何回写
- 流式事件是否能和工具事件对齐

这些都属于原生协议层信息，不应该一直被中间层遮掉。

### 3. Pony 内部抽象会被过渡层反向绑架

如果中间协议留太久，内部结构很容易围着它长，最后反而更难切到真正的 provider protocol。

所以最小闭环验证完之后，应该尽快进入原生协议阶段。

## 为什么切到原生协议后，仍然要保留 Pony 自己的内部抽象

这是最关键的一点。

切到 OpenAI / Anthropic 原生 tools 协议，并不等于让 Pony 内部直接长成某一家厂商 API 的形状。

更合理的分层是：

### 1. Pony 内部统一抽象

这里放：

- `ToolCall`
- `ToolResult`
- `ToolRouter`
- runtime 的 trace / tool event

### 2. Provider adapter

这里负责把外部协议映射进内部抽象：

- OpenAI `tool_calls -> ToolCall`
- Anthropic `tool_use -> ToolCall`
- `ToolResult -> tool message / tool_result`

这样才能既接真实协议，又不把核心 runtime 写死在某一家 provider 上。

## 当前代码意味着什么

截至这一轮，Pony 的 tool 层已经进入了一个更健康的状态：

### 内部

- 有统一的 `ToolCall / ToolResult / ToolRouter`
- runtime 只关心“要执行哪个工具、返回什么结果”

### 外部

- OpenAI 走 `tool_calls -> tool`
- Anthropic 走 `tool_use -> tool_result`

### 前端

- 继续消费统一的 `turn:trace / turn:tool / turn:delta / turn:completed`

也就是说，UI 不需要知道 provider 的底层工具协议长什么样，但 runtime 和 provider adapter 已经知道。

## 这对 Pony 的架构方向意味着什么

这一步非常重要，因为它说明 Pony 已经不再只是“能流式聊天的桌面工作台”，而是开始具备更真实的 agent core 分层：

- runtime loop
- tool router
- provider adapter
- event bridge

这正是后面走向独立 Rust core、HTTP/SSE adapter、session persistence 的前提。

## 当前还没有的部分

虽然已经切到原生 tools 主路径，但这不等于工具层已经完成。

当前还没补齐的主要有：

1. 更多工作区工具，比如列文件、分段读文件
2. 更明确的工具错误态与恢复语义
3. 多工具策略
4. 并发工具策略
5. 更严格的工具权限边界

所以这一步更准确地说，是“原生 tools 协议闭环成立”，而不是“工具系统完成”。

## 当前学习进度

截至这一轮，可以明确地说：

1. Pony 已经验证过最小 tool 闭环
2. 现在 live provider 主路径已经切到 OpenAI / Anthropic 原生 tools 协议
3. Pony 内部仍保留统一 `ToolCall / ToolResult / ToolRouter` 抽象
4. 这说明当前方向已经从“验证骨架”进入“真实协议接入”

## 下一步最自然的学习方向

接下来最自然的学习和开发顺序是：

1. 继续补工具层边界，比如 `workspace.list_files`、`workspace.read_file_segment`
2. 收紧工具错误态与 tool event 语义
3. 再补运行时指标，比如 `token 统计 / 首 token 延迟`
4. 之后再进入 session store 和独立接入层

## 一句总结

最小 tool 闭环的意义，是先证明 Pony 的 agent loop 已经长出来；切到 OpenAI / Anthropic 原生 tools 协议的意义，是让这个骨架真正接上现实世界的 provider runtime。
