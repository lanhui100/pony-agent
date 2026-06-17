# PA-055 Code Review

## 审核对象

- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [runtime/mod.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/runtime/mod.rs)
- [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)

## 审核方式

- 使用 `opencode run`
- 模型：`opencode/deepseek-v4-flash-free`
- 方式：并行发起 2 份只读代码审核
  - 实现正确性 / 回归风险 / 测试缺口
  - 默认工具合同一致性 / 命名语义 / schema 对齐

## 审核结果概览

- 实现向审核：已获得有效结论
- 合同向审核：已通过 `opencode export` 成功导出最终文字结论

## 实现向高优先级意见

1. `Ask` 从纯文本输出变为 JSON 后，可能影响仍直接消费 `result.output` 纯文本的下游。
2. `Run` 将 `ToolResult.tool_name` 改为 `run_shell` 有直接回归风险，容易击穿对 `workspace_run_command` 的等值判断。
3. `Plan` 在 runtime 中走了一条硬编码 `workspace.read` / `requires_approval=false` 的特殊分支，权限语义不安全。
4. `Ask` 如果把漏参也视为 success，会掩盖模型侧参数错误。
5. `Plan` 新增路径只有 happy path 测试，缺少边界与错误传播覆盖。
6. `ToolSearch` 同时输出 flat `sourceId` 与 nested `source.sourceId`，需要标记兼容目的。

## 已采纳项

1. 撤回 `Run` 对 `ToolResult.tool_name` 的破坏性改动，恢复为 `workspace_run_command`，仅在 payload 中增加 `delegateTool=run_shell`。
2. 撤回 `Plan` 在 runtime 中的硬编码特殊执行分支，避免绕过既有 capability / tool execution 路径。
3. 收紧 `Ask` fallback：只有显式提供 `question` 时才走 `fallback_clarification`；完全缺参时恢复 error。
4. `Ask` 正常 `text` 路径恢复为旧的纯文本 `output`，降低对直接消费 `result.output` 的下游回归风险。
5. 为 `Ask` 新增“正常 echo / fallback / 完全缺参 error”三类测试。
6. 为 `ToolSearch` 的 flat + nested `source` 双输出补兼容注释，明确是过渡结构。

## 暂未采纳项

1. 暂未对 `ToolSearch` flat 字段给出明确移除时间表或 deprecation 计划。
   原因：当前更适合先稳定第三波兼容输出，再在后续 contract cleanup 卡里做精简。
2. 暂未对合同向审核中提到的 `ToolSearch` 字段冗余做进一步收缩。
   原因：当前输出同时承担 spec 字段补齐与向后兼容职责，适合在后续 cleanup 卡中处理。

## 合同向审核结论

合同向审核对 `Plan / Ask / ToolSearch / Run` 的总体结论是“基本符合第三波 spec”，其中明确指出：

1. `Plan` 的默认工具身份与 `ToolPlan / child_results` 暴露方式符合 spec。
2. `Ask` 的 `question` fallback 与副作用边界符合 spec。
3. `Run` 保持 canonical product name 为 `Run`，内部 `RunShell` 仅作为委托实现语义存在。
4. `ToolSearch` 已包含 spec 要求的 `tool_name / description / source / confidence`。
5. 唯一明确的可改进项仍是 `ToolSearch` 结果里 top-level 与 nested `source` 字段并存，存在冗余。

## 合同向采纳情况

1. 已确认 `Plan / Ask / Run` 三项合同向问题在后续代码调优中已收口：
   - 已撤回 `Plan` 的硬编码权限特殊分支
   - `Ask` 正常路径恢复纯文本兼容输出，fallback 仅在显式 `question` 时触发
   - `Run` 恢复 `tool_name=workspace_run_command`，仅在 payload 中暴露 `delegateTool=run_shell`
2. `ToolSearch` 字段冗余问题暂保留为兼容输出，不在本卡继续收缩。

## 验证结果

已完成以下验证：

1. `cargo check -p pony-agent-core`：通过
2. `cargo test -p pony-agent-core ask_returns_fallback_clarification_when_text_missing -- --nocapture`：通过
3. `cargo test -p pony-agent-core ask_keeps_plain_text_output_for_normal_echo_path -- --nocapture`：通过
4. `cargo test -p pony-agent-core ask_returns_error_when_text_and_question_both_missing -- --nocapture`：通过
5. `cargo test -p pony-agent-core tool_search_returns_registry_tool_candidates -- --nocapture`：通过

验证过程中存在 Windows 增量编译目录 `os error 5` warning，但未影响本轮编译或测试结论。

## 审核证据

- 实现向审核输出：
  [.tmp/pa055-code-review-impl.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa055-code-review-impl.jsonl)
- 合同向审核事件流：
  [.tmp/pa055-code-review-contract.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa055-code-review-contract.jsonl)
- 合同向会话导出：
  `opencode export ses_12f7fab08ffebGnjiUY0I3pWjU`
- 实现向会话导出：
  `opencode export ses_12f7fb9a0ffeQe7fCZVTWKN1im`
