# Design: Redesign Context Assembly And Cache Strategy

## 背景

当前项目在 [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/context.rs) 中已经实现了第一版 `TurnContextBuilder`，但真实请求仍以“每轮重建完整上下文”为主：

- `system + capability note + semistable context + history + user`
- 若是 provider-native tool flow，则重建 `system + capability + semistable + transcript + user`

问题不在于有没有 `stable_prefix_text` 观测，而在于真正发送给 provider 的前缀仍被半稳定内容污染。

## 设计目标

1. 明确稳定前缀与半稳定上下文的正式边界
2. 为 `system prompt / runtime facts / AGENT.md / workspace / memory / conversation carry` 建立单一分层模型
3. 让后续实现能在不重复改动架构文档的前提下推进
4. 把 cache-reset 收敛为低频、显式、可观测事件
5. 让长期记忆未来能接入，但不污染本轮稳定前缀

## 非目标

- 本轮不交付长期记忆写入、召回、排序产品能力
- 本轮不重写全部 provider transport
- 本轮不要求完整 UI 切换 coding/work profile
- 本轮不为每种 surface 单独生成新 prompt 模板文件

## 分层模型

### Layer 0. Tools

工具定义、顺序、描述文本属于缓存最敏感层之一。

约束：

- tool schema 顺序 SHALL 稳定
- 同一线程内 tool 描述文本 SHALL NOT 因 session 状态变化而改写
- deferred / dynamic tool discovery 应通过 `ToolSearch` 等能力显式处理，而不是每轮改写整个 tool surface

### Layer 1. Base System

只承载最稳定的 agent 身份、行为规范、安全约束、输出规范。

约束：

- `Base System` SHALL NOT 直接携带 session summary、run goal、history truncation note 一类动态内容
- `Base System` SHOULD 拆成 `coding` 与 `work` 两类 profile
- 同一线程中 profile 一旦确定，除显式模式切换外 SHALL 保持稳定

### Layer 2. Runtime Facts

承载线程级稳定环境事实，例如：

- OS
- shell
- surface（desktop / cli / http / tui）
- sandbox / approval
- workspace roots

约束：

- runtime facts 可以进入稳定前缀，但前提是它们在一个线程内基本稳定
- 频繁变化的 runtime diagnostics SHALL NOT 进入本层

### Layer 3. Project Instructions

承载 `AGENT.md / AGENTS.md / 类 CLAUDE.md 项目说明` 一类项目级指令。

约束：

- `AGENT.md` SHALL NOT 进入 base system 本体
- 它属于 project instructions 层
- 需要作用域、覆盖优先级和路径匹配规则

### Layer 4. Memory Injection

承载未来跨 session 长期记忆的注入策略。

本轮只定义接口，不定义完整产品行为。

排序理由：

- `Memory Injection` 表示来自线程外、可跨 session 复用的持久信息来源
- `Conversation Carry` 表示当前线程内部对话如何续写、压缩和重放

因此这里按“来源边界”而不是按“短期稳定性强弱”排序：先线程外持久上下文，再线程内会话续写。

### Layer 5. Conversation Carry

承载会话续写策略，包括：

- provider continuation
- compaction
- stable history replay

核心目标是尽量减少“每轮全量重放”。

### Layer 6. Turn-local Volatile Input

承载当前轮用户输入、附件、一次性诊断说明与临时提示。

约束：

- 这一层必须位于最末尾
- 本层内容可以高频变化，不要求缓存命中

## System Prompt Builder

### 1. 双 Profile

建议建立：

- `BaseSystemPrompt::Coding`
- `BaseSystemPrompt::Work`

二者共享统一骨架，但在以下方面允许不同：

- 工具使用偏好
- 输出风格
- 任务推进方式
- 验证/审阅要求

### 2. 环境信息位置

环境信息不应全部塞进 `Base System`。

建议：

- 稳定人格与行为规范进入 `Base System`
- 线程级环境事实进入 `Runtime Facts`

这样可以避免：

- 为了适配不同 OS / surface 重建整个 base system
- coding/work profile 与环境事实交叉乘法爆炸

## AGENT.md 与 Workspace Instructions

### 1. 作用域规则

参考 `Codex`，采用目录树作用域：

- `AGENT.md` 的作用域是其所在目录及所有子目录
- 更深层文件覆盖更浅层文件
- system / developer / user 指令优先于 `AGENT.md`

### 2. 注入时机

默认注入：

- workspace root 的 `AGENT.md`
- 从 active cwd 到 workspace root 路径链上的 applicable instructions

增量刷新触发条件：

- active cwd 穿越新的 instruction scope
- target file 进入更深层作用域
- 用户显式切换 workspace

### 3. Workspace Binding

必须显式建模：

- `workspace_roots`
- `active_cwd`
- `target_paths`
- `applicable_instruction_sources`

前端和 runtime 不允许仅根据当前目录字符串隐式猜测 instruction scope。

### 4. 覆盖语义

`AGENT.md` 深层覆盖采用文件级覆盖优先，而不是声明级合并。

理由：

- 文件级覆盖更容易稳定实现和观测
- 避免把冲突判定变成段落级或主题级启发式解析
- 更符合缓存友好目标：减少隐式拼接与不稳定组合

## Memory Extension Points

本轮先定义接口：

- `MemoryProvider`
- `MemorySelectionPolicy`
- `MemoryInjectionMode`

建议支持三种注入模式：

- `none`
- `instruction_tail`
- `retrieval_block`

示意签名：

```rust
trait MemoryProvider {
    fn recall(&self, query: &MemoryRecallQuery) -> MemoryRecallResult;
}
```

约束：

- 长期记忆默认 SHALL NOT 直接进入 base system
- 只有被判定为 durable policy 的低频稳定记忆，未来才允许提升到更稳定层

## Conversation Carry Strategy

### 1. 优先 continuation

若 provider 支持 continuation / `previous_response_id` 一类机制，应优先使用它，而不是每轮全量重放历史。

默认回退策略：

- continuation 失败时 SHALL 回退到显式 full replay
- 回退时 SHALL 记录 observable reason
- 不允许静默从 continuation 路径退化为隐式重放

### 2. 低频 compaction

若必须改变历史，应通过低频 compaction 显式完成。

compaction 是允许的 cache-reset 点，但应满足：

- 低频
- 可观测
- 有明确原因

### 3. 禁止高频前置动态说明

以下内容 SHALL NOT 每轮前置注入到稳定前缀中：

- session summary
- truncation note
- planner skills 变化摘要
- temporary diagnostics

它们要么后置，要么只在 compaction / refresh boundary 中显式重写。

建议默认分配：

- `session summary` -> `Conversation Carry`
- `truncation note` -> `Conversation Carry`
- `planner skills summary` -> `Project Instructions` 的按需刷新片段
- `temporary diagnostics` -> `Turn-local Volatile Input`
- `single-turn user instruction overrides` -> `Turn-local Volatile Input`

## 可观测性要求

构建层必须继续保留并强化以下观测：

- stable prefix text
- semi-stable context text
- volatile input text
- prefix mutation reasons
- request kind
- cache hit / miss tokens

并补充：

- `context_refresh_reason`
- `instruction_scope_sources`
- `conversation_carry_mode`

## Migration Path

### 当前实现到目标分层的映射

| 当前来源 | 目标层 |
|---|---|
| `BASE_SYSTEM_PROMPT` | `Base System` |
| `provider_capability_note(...)` | `Runtime Facts` 或稳定 capability facts |
| `provider_semistable_context_note(...)` 中的 session summary / truncation note | `Conversation Carry` |
| `planner_skills` 摘要 | `Project Instructions` 的按需刷新片段 |
| `long_term_memory` 摘要 | `Memory Injection` |
| `history_messages` / `native transcript` | `Conversation Carry` |
| 当前 user message / images / 单轮临时指令 | `Turn-local Volatile Input` |

### 第一阶段实现切入点

1. 在 `context.rs` 中引入显式 layered context 结构，而不是继续直接拼 `messages`
2. 在 `provider.rs` 中让 request observation 与新结构对齐
3. 在观测面新增 `context_refresh_reason / instruction_scope_sources / conversation_carry_mode`

## 审核要求

本轮文档必须完成至少一轮 `opencode / deepseek-v4-flash-free` 独立只读审核，并至少覆盖以下 3 个维度：

1. 分层合理性与任务拆分
2. 缓存边界与前缀稳定性
3. `system prompt / AGENT.md / workspace / memory hooks` 一致性
