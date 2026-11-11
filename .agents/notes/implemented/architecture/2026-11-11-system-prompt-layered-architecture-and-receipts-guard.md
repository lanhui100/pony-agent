# Agent Note: 系统提示词分层架构、收据化原则与层序守卫

Status: implemented

## Problem

随着项目演进，底层系统提示词（`BASE_SYSTEM_PROMPT`）经历了多次增补，产生了四类结构性债务：

1. **与工具说明书冗余重复**：例如 `todo_write` 等工具规则在提示词和工具 schema 中双重陈述，消耗有限上下文预算；
2. **缺乏不可信外部数据防线**：工具执行回显和网页抓取内容直接进入上下文，缺乏统一安全声明，存在 Prompt Injection 攻击面；
3. **指令阈值冲突**：系统提示词内部与工作区指令存在文本长度和规则阈值打架，导致大模型决策混乱；
4. **提示词层序缺乏硬校验**：静态系统提示词（System）与半动态指令（Developer/Workspace）的组装顺序若被打乱，会导致大模型服务商前缀缓存（Prompt Cache）失效，造成显著延迟与费用上升。

## Decision

1. **客观机器收据优先（Objective receipts）**：
   - 确立「以物理证据为准：命令退出码、测试断言、编译器输出」为第一行动准则，严禁凭假设推进，遇非零退出必须先排查自愈。
2. **注入防线（Untrusted Data Boundary）**：
   - 明确将工具回显和抓取内容定义为不可信数据，严禁直接作为指令执行，发现疑似注入主动向用户求证。
3. **阈值冲突收敛与 Stricter wins 原则**：
   - 决策说明严格压缩至 ≤2 行；当工作区指令存在更严格约束时，执行从严原则。
4. **提示词层序可测守卫**：
   - 引入硬约束 `prompt_layers_are_ordered`：所有 System 角色消息必须排在所有 Developer/User 消息之前，首条必须为基座 System Prompt；
   - 在上下文构建管线中加入断言守护，保护前缀缓存命中率（ADR 0007 核心指标）。
5. **历史溯源与接替**：
   - 本决策源自并完全接替历史决策 `docs/archive/decisions/0044-system-prompt-v2-receipts-injection-and-layer-order.md`。

## Alternatives considered

- **完全从系统提示词中剥离规划类工具约束**：
  将所有工具说明完全交由 schema 承载。
  *落选原因*：在长上下文复杂会话中，模型容易忽略复杂规划工具的存在，提示词中保留一句话触发约束是最佳平衡。
- **将半动态层降级为 User 角色消息**：
  *落选原因*：Anthropic 等协议下 Developer 消息与 System 消息在协议层有特定映射，随意修改角色会打破跨服务商适配契约。

## Consequences

- 消除系统提示词约 120 字节冗余文本；
- 确立了机器收据与注入防御的基准原则；
- 层序校验成为自动化测试（`system_prompt_contract.rs`）可断言的物理契约，防止前缀缓存劣化。
