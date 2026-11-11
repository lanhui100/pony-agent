# PA-056 Code Review

## 审核对象

- [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/context.rs)
- [provider.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/provider.rs)
- [turn_flow.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/turn_flow.rs)
- [runtime/mod.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/runtime/mod.rs)
- [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)
- [HomeSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSidebar.vue)
- [tests/runtime-store.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/runtime-store.spec.ts)
- [tests/HomeSidebar.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeSidebar.spec.ts)

## 审核方式

- 使用 `opencode run`
- 模型：`deepseek/deepseek-v4-flash`
- 方式：3 轮独立只读代码审核
  - 实现正确性与分层语义
  - 兼容性、回归风险与观测一致性
  - Implementation Bridge 完整度与后续演进风险

## 审核结论

- 3 轮总体结论均为：`Conditionally Pass`

## 主要问题

1. `native_memory_messages` 初版实现为空，导致 native tool flow 下 memory layer 丢失。
2. `Long-term memory` 同时进入 `project_instruction_messages` 与 `memory_messages`，存在重复注入。
3. `derive_context_refresh_reason` 初版带有伪 diff 语义，容易把普通后续构建误标为 `InitialBuild` 或静态“Changed”。
4. `volatile_input_text` 初版在 normalized 路径退化为纯 message 渲染，丢失图片元信息。
5. 新增观测字段存在测试覆盖不足，尤其是 native memory、refresh reason 与 instruction scope sources。

## 已采纳项

1. 补齐 native 模式下的 memory layer，新增 `build_native_memory_messages(...)`，确保 `request.native_messages` 不再丢失长期记忆注入。
2. 从 `provider_semistable_context_note(...)` 中移除长期记忆注入，改为只通过 `Memory Injection` 层承载，消除 LTM 双重注入。
3. 收紧 `derive_context_refresh_reason(...)` 语义：
   - 首轮构建才给 `InitialBuild`
   - 普通 follow-up 在无特殊触发时返回 `None`
   - 不再用 `session summary 非空` 冒充“发生变化”
4. 为 normalized 路径恢复专门的 `volatile_input_observation_text`，保留图片元信息观测。
5. 补充 Rust 测试，覆盖：
   - normalized/native observation
   - native memory layer 注入
   - plain follow-up 下 `context_refresh_reason == None`
   - instruction scope sources / carry mode 基本落点
6. 同步前端类型与展示，新增 `contextRefreshReason / instructionScopeSources / conversationCarryMode` 字段兼容。
7. 修复 `HomeSidebar` 的 `CALL MODEL` 展开态指标，补 `耗时` 明细显示，恢复测试通过。

## 暂未采纳项

1. 暂未把 `instruction_scope_sources` 从占位实现升级成真实的 workspace / AGENT.md source 枚举器。
   原因：本轮目标是先打通 Implementation Bridge 和观测落点，真实 instruction scope diff 需要结合后续 workspace/instruction builder 改造继续推进。
2. 暂未删除 `ConversationCarryMode` 中尚未落地的未来变体。
   原因：spec 已经为 continuation / compaction 预留扩展位，本轮保留枚举面，后续实现到位时再接通。
3. 暂未把 observation fallback 启发式路径改成强约束或硬错误。
   原因：当前仍需要兼容仓库内其它非 layered 构造路径，后续可单开 cleanup 收紧。

## 审核证据

- 第一轮：
  [.tmp/pa056-code-review-1.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-code-review-1.jsonl)
- 第二轮：
  [.tmp/pa056-code-review-2.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-code-review-2.jsonl)
- 第三轮：
  [.tmp/pa056-code-review-3.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-code-review-3.jsonl)
- 当前 diff：
  [.tmp/pa056-current.diff](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-current.diff)

## 2026-06-17 Follow-up

- 后续补强与二次采纳见：
  [2026-06-17-pa056-code-review-followup.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-17-pa056-code-review-followup.md>)
