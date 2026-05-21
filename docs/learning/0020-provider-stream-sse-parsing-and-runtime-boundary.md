# 0020 Provider 流式 SSE 解析与 runtime 边界

## 这次学习在回答什么
这次不是在学一个具体函数怎么写，而是在回答一个更关键的问题：

当我们说“已经接入真实 provider stream”时，到底意味着什么？

更准确地说，它至少包含两层：

1. 外部 provider 的真实协议已经被正确消费
2. 内部 runtime 看到的已经是被统一过的结果，而不是原始协议碎片

这次联调里暴露的问题，刚好说明了这两层不能混在一起。

## 这次故障的本质
表面现象是：

- 用户发出请求
- provider 确实返回了内容
- 但系统报错说“解析 provider 返回失败”

如果只看最后的错误，很容易误以为是：

- 工具调用失败
- 模型质量不稳定
- 网络偶发错误

但这次真正的根因不是这些，而是：

OpenAI 兼容接口在 `stream=true` 时返回的是 SSE 文本流，而不是普通 JSON。

也就是说，外部协议长这样：

- 一行一行的 `data: {...}`
- 最后以 `data: [DONE]` 结束

如果内部还沿用“整包 JSON 反序列化”的思路，就一定会在第一行 `data:` 上失败。

## 为什么这件事重要
因为它帮助我们把三层问题分开：

### 1. provider 协议层
这一层负责理解外部世界到底怎么说话。

比如：

- OpenAI 兼容 stream 是 SSE chunk
- Anthropic 可能是另一种事件组织形式
- 非 stream 请求可能还是普通 JSON

这一层的责任，是把“厂商协议”翻译成 Pony 内部能理解的统一形式。

### 2. runtime 层
这一层不应该关心 SSE 长什么样，也不应该自己解析 `data: ...`。

它关心的是：

- 有没有增量文本
- 有没有 tool call
- 有没有 fallback
- token、trace、phase 怎么更新

也就是说，runtime 消费的是“统一后的内部语义”，而不是外部协议细节。

### 3. tool 层
这层只在 tool call 已经成立时才接手。

所以当请求在 provider stream parse 阶段就失败时，问题还没进入 tool 层。

这正是为什么这次要明确说：

这不是工具问题，而是 provider 适配问题。

## 它体现了什么设计理念
这次故障非常适合用来理解一句很重要的话：

不要让领域核心直接暴露在协议噪音里。

这里的“领域核心”是 runtime。
这里的“协议噪音”是外部 provider 的原始 SSE 文本格式。

如果 runtime 直接去理解这些细节，会带来几个坏处：

- runtime 越来越像某一家 provider SDK
- 每加一个 provider 协议，runtime 就再多一层分支
- 错误定位会越来越混乱，分不清是协议问题、业务问题还是工具问题

所以更健康的做法是：

- provider adapter 负责吃掉协议差异
- runtime 只消费统一后的增量文本、tool call、usage 和错误信号

## 这对我们后续重构有什么影响
这次修复不是一个孤立 bugfix，它会直接影响后面的几条路。

### 1. 独立 adapter 的必要性更清楚了
如果未来要支持 Tauri、Web、Linux 服务，那么我们不只是要拆 Tauri adapter，也要继续保持 provider adapter 的清晰边界。

这就是为什么“独立接入层”是接下来自然的一条并行任务线。

### 2. 工具层不该背协议锅
工具层后续当然还要继续扩展，但它的前提是 provider 已经把 tool call 正确翻译成内部数据结构。

否则我们会一边扩工具，一边被协议错误误导。

### 3. 可观测性要继续往 provider 层收口
这次问题能快速定位，是因为已经开始重视：

- `provider_mode`
- `fallback_reason`
- 真正的原始响应

以后像 token、首 token 延迟、provider source 这些指标，也应该优先被视为 provider/runtime 可观测性的一部分。

## 当前阶段可以记住的结论
1. “真实 stream 已接通”不只是能收到字，而是要能正确消费 provider 的真实协议
2. SSE 解析属于 provider adapter 责任，不属于 tool 层，也不应该落到 runtime 主逻辑里
3. 这次修复进一步证明，我们现在的下一步不只是补 UI，而是继续收束 agent core 的边界
4. 后续很适合并行推进两条线：独立接入层、工具层补强

## 可延展选题
- OpenAI 兼容 stream、Responses API、Anthropic event stream 在适配层上的共同点与差异
- 为什么“协议适配”和“领域 runtime”应该分层
- 真实联调中，如何区分网络问题、协议问题、工具问题和 session 问题
