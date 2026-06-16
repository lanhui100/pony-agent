# Proposal: Redesign Context Assembly And Cache Strategy

## Why

Pony Agent 已经具备第一版上下文观测能力，但请求构建层仍把大量“会变化的说明性文本”放在请求前部，导致缓存命中长期偏低。

当前主要问题不是模型不支持缓存，而是我们的上下文组装方式让真正稳定的前缀过短：

- `Session summary / Run goal / Long-term memory / Planner skills / Truncation note` 每轮都可能变化
- 这些内容仍被前置到 history 和当前 user input 之前
- provider-native transcript follow-up 仍偏向全量重放
- `AGENT.md`、workspace scope、system prompt profile、长期记忆入口还没有统一分层

如果继续单点修补，很容易重复修改 `context builder / provider request / task card / spec` 多个层面，最终既损失缓存命中，也让架构边界持续漂移。

## What Changes

- 建立正式的上下文分层架构：
  - `Tools`
  - `Base System`
  - `Runtime Facts`
  - `Project Instructions`
  - `Memory Injection`
  - `Conversation Carry`
  - `Turn-local Volatile Input`
- 明确 coding / work 双 profile 的 system prompt builder 方向
- 明确 `AGENT.md` 与 workspace scope 的作用域、覆盖规则和注入时机
- 明确长期记忆未来如何接入，但只预留扩展点，不在本 change 实现记忆本体
- 明确哪些变化允许触发低频 cache-reset，哪些变化必须后置或按需注入

## Why This Grouping

这次工作不是分别优化几个分散字段，而是在统一回答同一个问题：哪些上下文能进入稳定前缀，哪些上下文必须后置、按需注入或通过低频 boundary 重写。

这条主线同时约束：

- `system prompt` 如何保持稳定
- `AGENT.md / workspace instructions` 如何进入上下文而不污染 base system
- `conversation carry` 如何续写、压缩和复用
- `memory injection` 如何为未来扩展预留入口而不提前破坏缓存边界

因此更适合用一个主 change 统一收口，而不是拆成多个最终又会反复修改同一请求构建层的子 change。

## Why One Change

这次不是 4 个松散需求，而是同一条“上下文构建主线”的 4 个侧面：

- system prompt 太薄且未 profile 化
- `AGENT.md` / workspace instructions 缺少正式层级
- conversation carry 与缓存边界未正式拆开
- 长期记忆若提前接入，当前前缀还会继续失稳

如果拆成多个独立 change，会导致：

- 多份 spec 重复定义上下文层级
- 一个 change 调整 system prompt，另一个再改 instruction 注入
- conversation carry 与 memory injection 在不同 change 中重复收口

因此这次应以一个主 change 统一定义架构，再在实现阶段拆任务。

## Out Of Scope

- 直接交付完整长期记忆产品功能
- 直接重做 Browser / Thread / Automation / Workflow 独立工具面
- 直接重写所有现有 provider 适配器
- 在本轮里完成所有 coding/work 模式的前端切换交互
