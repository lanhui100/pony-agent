# ADR-0026: 基础工具体系加固（CAS 乐观并发写守卫与富语义错误自愈协议）

## 状态
Accepted (已接受)

## 上下文
在多智能体（Agent Teams / Subagents）协作或快速交互场景下，Pony Agent 的内置文件工具（`workspace_write_file`, `workspace_edit_file`）缺乏防踩踏保护。对比 DSH 的 `FS_STALE_VERSION` 与 Claude Code 的差异化重基机制，Pony Agent 存在脏写覆盖与报错无自愈引导的缺陷。

## 决策
1. **轻量无状态 Content-Hash CAS**：
   - 在 `workspace_write_file` 与 `workspace_edit_file` 中新增可选参数 `expected_hash`。
   - 在 `workspace_read_file`、`workspace_write_file`、`workspace_edit_file` 返回结果中透出 `content_hash` (SHA-256)。
   - 当 `expected_hash` 存在且与当前磁盘内容计算的 SHA-256 不一致时，拒绝写入，返回 `cas_conflict`。
2. **结构化富语义错误与自愈引导（Actionable Recovery Hint）**：
   - 错误载荷不仅包含 `code` 和 `message`，还在 `details` 中注入 `recovery_hint`，包括 `action: "rebase_and_retry"` 与 `next_suggested_call`。
3. **保持向前兼容（Backward Compatibility）**：
   - 未传递 `expected_hash` 时，维持原生无锁/强写行为，保证历史调用链与旧 Prompt 零破坏。

## 非目标 (Non-Goals)
1. 不实现复杂的分布式锁服务或租约管理器。
2. 不在 Rust 工具层执行自动 3-way merge，冲突裁决权下沉给 LLM。
3. 不对只读或执行类工具强制施加版本控制。
