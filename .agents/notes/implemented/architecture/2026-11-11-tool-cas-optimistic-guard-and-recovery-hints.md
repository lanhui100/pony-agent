# Agent Note: 基础文件工具 CAS 乐观并发写守卫与富语义错误自愈协议

Status: implemented

## Problem

在多智能体（Agent Teams / Subagents）协作或长链路快速工具调用场景下，Agent 的内置文件操作工具（`workspace_write_file`、`workspace_edit_file`）面临写冲突与并发踩踏隐患：

1. **脏写与盲写覆盖**：多个智能体并发读写同一文件，或同一智能体在陈旧文件快照基础上发起编辑时，缺少轻量校验机制，容易无声覆盖他方变更；
2. **缺乏结构化自愈引导**：当并发冲突或写入失败时，工具层仅返回简单文本错误（如 "write failed"），LLM 缺乏明确动作指引（Actionable Recovery Hint），容易陷入重复尝试相同调用的死循环；
3. **重型锁机制的副作用**：引入全局分布式锁或路径租约锁会带来死锁风险与心智开销，破坏本地桌面轻量运行原则。

## Decision

1. **轻量无状态 Content-Hash CAS**：
   - 在写与编辑工具中引入可选 `expected_hash` 参数（SHA-256）；
   - 在读、写、编辑工具返回值中统一回显当前文件的最新 `content_hash`；
   - 当调用方传入 `expected_hash` 且与磁盘当前内容哈希不一致时，严格拒绝写入并返回 `cas_conflict` 错误码。
2. **富语义自愈引导（Actionable Recovery Hint）**：
   - `cas_conflict` 错误返回体中包含明确的自愈建议：`action: "rebase_and_retry"` 与下一推荐动作（重新读取最新内容后重基应用变更）；
   - LLM 识别此结构化回执后可确定性执行重基策略。
3. **完全向后兼容**：
   - `expected_hash` 参数完全可选，未传入时保留原生无锁强写逻辑，历史调用与现有 Prompt 零破坏。
4. **历史溯源与接替**：
   - 本决策源自并完全接替历史决策 `docs/archive/decisions/0026-tool-cas-and-recovery-hint.md`。

## Alternatives considered

- **分布式租约与全局写锁**：
  在主进程或工具调度层为每个文件路径维护互斥锁或租约。
  *落选原因*：Pony Agent 属于单机本地多任务与多智能体协作环境，全局锁极大增加了死锁概率与进程间通信成本，破坏系统的轻量化与高韧性要求。
- **由 Rust 内核自动执行 3-way merge**：
  当发生内容冲突时，由工具底层尝试代码合并。
  *落选原因*：语义层面的代码合并裁决权属于大模型与人类，底层硬套 3-way merge 容易产生错误的语法或逻辑残留，违背 Fail-Closed 原则。

## Consequences

- 工具层具备无状态并发冲突拦截能力；
- 多智能体协作和长轮次执行中，文件冲突能够被模型自愈闭环；
- 所有相关工具测试均覆盖了 CAS 命中、哈希不匹配阻断以及自愈重试逻辑。
