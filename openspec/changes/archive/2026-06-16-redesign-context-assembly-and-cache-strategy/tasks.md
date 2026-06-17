# Tasks: Redesign Context Assembly And Cache Strategy

## 1. Spec And Task-System Alignment

- [x] 1.1 新增 `PA-056` 任务卡，明确上下文构建架构改造的目标、范围和验收标准
- [x] 1.2 建立 `redesign-context-assembly-and-cache-strategy` OpenSpec change
- [x] 1.3 完成 proposal / design / spec / tasks 初稿

## 2. Layered Context Architecture

- [x] 2.1 定义 `Tools / Base System / Runtime Facts / Project Instructions / Memory Injection / Conversation Carry / Turn-local Volatile Input` 的正式分层
- [x] 2.2 明确哪些层允许进入稳定前缀，哪些层只能后置或按需注入
- [x] 2.3 明确允许的低频 cache-reset 点与观测口径

## 3. Prompt And Instruction Builder

- [x] 3.1 定义 `coding / work` 双 profile 的 system prompt builder 方向
- [x] 3.2 明确 `Runtime Facts` 与 `Base System` 的分离规则
- [x] 3.3 明确 `AGENT.md` / workspace instructions 的作用域、覆盖优先级与注入时机
- [x] 3.4 明确 workspace roots / active cwd / target paths / applicable instruction sources 的最小建模
- [x] 3.5 明确 `AGENT.md` 深层覆盖采用文件级覆盖，而不是声明级合并

## 4. Conversation Carry And Memory Hooks

- [x] 4.1 明确 provider continuation、compaction、full replay 的优先级
- [x] 4.2 明确禁止每轮前置注入的动态说明项
- [x] 4.3 为长期记忆预留 `MemoryProvider / MemorySelectionPolicy / MemoryInjectionMode` 扩展点
- [x] 4.4 明确本轮长期记忆非目标与后续演进边界
- [x] 4.5 明确 continuation 失败时回退到 full replay 的默认行为与可观测原因
- [x] 4.6 为 `session summary / truncation note / planner skills summary / temporary diagnostics` 写出显式层分配

## 5. Review And Tightening

- [x] 5.1 使用 `opencode / deepseek-v4-flash-free` 完成至少一轮独立只读 spec 审核
- [x] 5.2 审核至少覆盖 3 个维度，并形成独立 review 记录
- [x] 5.3 汇总采纳与不采纳项
- [x] 5.4 根据采纳意见调优 proposal / design / spec / tasks
- [x] 5.5 在任务卡中同步 spec 状态、review 路径与下一步动作

## 6. Implementation Bridge

- [x] 6.0 在实现前定义 `BaseSystemPromptBuilder` contract、profile selection policy 与切换时的 cache-boundary 行为
- [x] 6.1 在 `context.rs` 中引入 layered context 数据结构，替代继续直接扁平拼接 `messages`
- [x] 6.2 修改 `TurnContextBuilder`，把现有 `BASE_SYSTEM_PROMPT / semistable context / history / user input` 映射到正式分层
- [x] 6.3 修改 `provider.rs` 中 request observation 的来源，使其直接消费 layered context
- [x] 6.4 为 `context_refresh_reason / instruction_scope_sources / conversation_carry_mode` 补齐观测字段和落点
- [x] 6.5 打通显式 `workspace_mode` 配置主链：`AppSettings -> Tauri commands -> frontend settings store -> submitTurn -> TurnContext -> domain profile`
- [x] 6.6 在左侧边栏尾部补设置入口，并新增 `Coding / Work` 设置面板
- [x] 6.7 补最小回归：settings store、settings 页面切换、sidebar 设置入口、`workspaceMode` 提交链路
