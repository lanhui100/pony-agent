# Pony Agent 上下文压缩设计原则

## 文档目的

这份文档描述 Pony Agent 当前已实现的上下文压缩策略，以及指导后续演进的设计原则。

核心目标不是单纯缩短上下文，而是在以下目标之间取得平衡：

- 保留足够任务信息
- 保持前缀稳定
- 提高 cache 命中率
- 降低延迟与成本

## 当前实现（2026-07）

### 架构位置

压缩逻辑位于独立模块 `crates/pony-agent-core/src/agent/compression.rs`，通过 `CompressionConfig` 暴露可配置参数：

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `trigger_threshold` | 0.8 | 触发压缩的阈值，占 context window 的比例 |
| `keep_recent_turns` | 10 | 保留原始消息的最近 turn 数 |

### 触发时机

在 `prepare_turn()` 中、构建上下文之前触发：

```
用户输入
  ↓
resolve_provider()
  ↓
获取 session 快照
  ↓
should_compress() 检查总 token > 80% context_window？
  ├─ 否 → 跳过压缩，继续正常流程
  └─ 是 → 执行压缩
       ├─ split_history() 拆分为 [可压缩部分, 保留部分]
       ├─ 调用 LLM 生成结构化摘要
       ├─ apply_compression() 替换旧消息
       └─ replace_session_history() 更新会话
  ↓
构建上下文（此时历史已压缩）
```

### 压缩流程

```
原始历史: [msg1, msg2, ..., msgN]  (N 轮对话)
  │
  ├─ to_compress = msg[0..split]   (前 N-10 轮)
  └─ to_keep     = msg[split..]    (最近 10 轮)
       │
       ▼
  LLM 压缩调用（system prompt + to_compress 消息）
       │
       ▼
  摘要文本 → wrap_summary_to_message()
       │
       ▼
  新历史: [summary_message, ...to_keep]
```

### 摘要结构

LLM 生成的摘要使用以下 XML 格式：

```xml
<session_summary>
  goal:      会话的总体目标
  progress:  已完成的关键工作
  decisions: 架构/技术决策和用户偏好
  files:     涉及的关键文件及变更摘要
  issues:    遇到的错误和解决方案
  pending:   明确的待办事项
  state:     压缩前一刻正在做的事情
</session_summary>
```

摘要注入历史时，用前缀和后缀包裹：

```
The following is a compressed summary of earlier conversation...
<session_summary>
  ...
</session_summary>
Continue directly from where the conversation left off...
```

### 压缩后下一轮的上下文

```
 1. [System]     BASE_SYSTEM_PROMPT
 2. [Developer]  Domain profile
 3. [Developer]  Project instruction
 4. [Developer]  Memory messages
 5. [Developer]  Volatile context
 6. [User]       摘要消息 ← 压缩后的旧对话
 7. [User]       第 N-9 轮用户消息  ← 保留的最近 10 轮
 8. [Assistant]  第 N-9 轮回复
     ...         ...
 9. [User]       当前用户消息
```

### 与 context.rs 的配合

`context.rs` 中的 80% 阈值检查作为第二道防线：

- 如果压缩成功 → 总 token 远低于 80%，保留全部历史
- 如果压缩失败（LLM 调用出错）→ 回退到 `truncate_history_messages` 按 token budget 截断
- 如果未触发压缩（低于 80%）→ 保留全部历史，不截断

## 总原则

### 1. 压缩不是越早越好

在上下文仍然健康时，优先保留原始消息结构，不要过早摘要。

原因：

- 原始历史往往更利于保持稳定前缀
- 过早压缩容易让前缀在每轮都变化
- 会降低 cache 复用价值

当前实现通过 **80% 阈值** 确保只在上下文接近窗口上限时才压缩。

### 2. 优先追求"稳定压缩"，而不是"频繁压缩"

一旦某段历史已经压成摘要，应尽量保持它稳定，不要下一轮再把同一段内容重新改写。

原因：

- 摘要一旦反复变化，token 前缀也会反复变化
- 这会直接破坏缓存命中

当前实现中，`split_history()` 保留最近 10 轮原始消息，只有更旧的消息才会被压缩。后续轮次中，摘要消息保持稳定，只有新的对话累积到超过 10 轮时才会触发新的压缩。

### 3. 压缩应尽量在明确边界发生

压缩应发生在稳定区段，而不是每一轮都重写整个历史。

推荐边界：

- 超过上下文阈值时（当前实现）
- 某段任务已经完成时
- 某一批工具调用已经闭环时
- 某个阶段性讨论已经结束时

### 4. 保持前缀分层

上下文应尽量拆成不同稳定度的层，而不是视为一个整体。

建议分层：

- 稳定层：system prompt、长期规则、固定摘要
- 半稳定层：最近若干轮关键对话
- 易变层：本轮工具结果、临时附件、即时提示

压缩时应优先只动某一层，避免整段重写。

当前实现中，压缩只替换历史层（conversation_carry），不动 system prompt、domain profile、memory 等稳定层。

### 5. 不要为了变短而损失可复用前缀

每次压缩设计都要先评估：

- token 变少了多少
- 前缀稳定性损失了多少
- 下一轮是否更难命中 cache

只有"综合收益"为正时，压缩才值得做。

## 具体策略

### 策略一：延迟压缩（已实现）

在上下文长度还没有逼近窗口时，不主动压缩。

当前实现：80% context window 阈值触发。

### 策略二：边界式压缩（已实现）

把较老、较稳定的一段历史一次性压成摘要，然后尽量固定住。

当前实现：`split_history()` 将旧消息一次性压缩，摘要消息固定后不再改写。

### 策略三：摘要冻结（已实现）

已生成的历史摘要，默认不在每轮继续重写。

当前实现：摘要消息作为一条 user 消息存入历史，后续轮次中保持不动，直到下一次压缩触发。

### 策略四：局部压缩优先于全局重写（部分实现）

优先只压缩：

- 工具结果
- 很长的附件
- 已完成分支
- 冗长但低复用的中间文本

当前实现：压缩整个旧历史段，尚未细化到按内容类型选择性压缩。

### 策略五：压缩结果需要可观测

压缩不是隐形动作，未来应能在 trace 或 runtime 状态中体现：

- 为什么压缩
- 压了哪一段
- 采用了哪种压缩方式
- 是否生成了新的稳定摘要

当前实现：压缩失败时打印错误日志，尚未在 trace 中暴露压缩事件。

## 代码入口

- 压缩模块：`crates/pony-agent-core/src/agent/compression.rs`
- 上下文构建：`crates/pony-agent-core/src/agent/context.rs`
- 运行时集成：`crates/pony-agent-core/src/agent/runtime/mod.rs`（`prepare_turn()`）
- 会话历史更新：`crates/pony-agent-core/src/agent/session.rs`（`replace_session_history()`）

## 一句话原则

好的上下文压缩，不是"尽量压短"，而是"尽量在不破坏稳定前缀的前提下压短"。