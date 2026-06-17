# PA-056 Code Review Follow-up

## 审核背景

- 时间：`2026-06-17`
- 方式：使用 `opencode run --model deepseek/deepseek-v4-flash --format json`
- 轮次：3 轮独立只读代码审核
- 审核对象：
  - [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/context.rs)
  - [runtime/mod.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/runtime/mod.rs)
  - [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)

## 审核维度

1. `BASE_SYSTEM_PROMPT` 与 domain profile 的兼容性与回归风险
2. 小上下文窗口下的缓存友好性与 token 预算风险
3. `workspace_mode` 贯通、inspection/handoff/persist 路径一致性，以及前端 runtime teardown 稳定性

## 主要意见

1. 高优先级：新 `BASE_SYSTEM_PROMPT` 移除了旧版“默认中文回复”语义，存在中文行为回归风险。
2. 高优先级：`coding / work` profile 文本过长，若每轮无条件注入，可能压缩小上下文模型的有效窗口。
3. 中优先级：未知 `workspace_mode` 被静默吞掉，调试时不易发现错误输入。
4. 中优先级：`inspect_retrieved_context_at / build_graph_turn_handoff / persist_turn_outcome` 等路径仍有 `workspace_mode=None` 回落到默认 `coding` 的风险。
5. 中优先级：前端测试虽然断言通过，但 teardown 后仍有 `window is not defined` 的异步未处理错误。

## 已采纳项

1. 恢复中文默认行为。
   - 在 [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/context.rs) 的 `BASE_SYSTEM_PROMPT` 中恢复 `Reply in Chinese unless the user explicitly requests another language.`
2. 压缩基础 prompt 与 profile 文本体积。
   - 将 `BASE_SYSTEM_PROMPT`、`CODING_DOMAIN_PROFILE_PROMPT`、`WORK_DOMAIN_PROFILE_PROMPT` 收紧为更短、更稳定的版本。
3. 为小上下文窗口跳过 domain profile 注入。
   - 新增 `MIN_CONTEXT_WINDOW_FOR_DOMAIN_PROFILE_TOKENS = 8192`
   - 新增 `build_domain_profile_messages(...)`
   - 当 provider `context_window_tokens < 8192` 时，不再把 `coding/work` profile 注入稳定前缀。
4. 为未知 `workspace_mode` 增加显式 fallback 提示。
   - 在 `infer_domain_profile_prompt(...)` 中输出 fallback 日志提示，避免静默吞掉。
5. 补齐运行时主链中的 `workspace_mode` 透传。
   - 将 turn 主执行、graph handoff、planner decision、turn outcome persistence 等真实运行路径接到显式 `workspace_mode`。
6. 修复测试环境 teardown 后的浏览器对象访问。
   - 在 [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts) 中新增 `resolveBrowserWindow()` 与 `safeSetTimeout()`
   - `wait()`、`waitForNextPaint()`、`runLowPriorityTurnWork()` 改为在异步触发时再次判定 `window`

## 未采纳或延后项

1. 未把所有 inspection/query 类接口都升级成显式携带 `workspace_mode` 的查询合同。
   - 原因：本轮优先修复真实 turn 执行链、graph handoff 和 persistence 的回落风险。
   - 处理：保留 query 型 inspection 默认回落行为，并在任务文档中标记为后续扩展点。
2. 未新增强制硬错误来拒绝未知 `workspace_mode`。
   - 原因：当前选择保留向后兼容的安全 fallback，仅增加可观测提示。

## 本轮新增验证

- `cargo check -p pony-agent-core --lib`
- `cargo test -p pony-agent-core build_request_ --message-format short -- --nocapture`
- `cargo test -p pony-agent-core retrieve_context_state --message-format short -- --nocapture`
- `cargo run -p pony-agent-core --bin non_tauri_harness`
- `npm exec vitest -- run tests/runtime-store.spec.ts tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/settings.store.spec.ts`

结果：

- Rust `build_request_` 相关 `12` 个测试通过
- Rust `retrieve_context_state` 相关 `2` 个测试通过
- `non_tauri_harness` 通过，输出 `non_tauri_harness ok`
- 前端 `145` 个测试通过，且已无 `window is not defined` 的 unhandled error

## 审核证据

- 第一轮：
  [.tmp/pa056-2026-06-17-code-review-1.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-2026-06-17-code-review-1.jsonl)
- 第二轮：
  [.tmp/pa056-2026-06-17-code-review-2.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-2026-06-17-code-review-2.jsonl)
- 第三轮：
  [.tmp/pa056-2026-06-17-code-review-3.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-2026-06-17-code-review-3.jsonl)
- 审核 prompt：
  [.tmp/pa056-review-prompt-1.txt](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-review-prompt-1.txt)
  [.tmp/pa056-review-prompt-2.txt](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-review-prompt-2.txt)
  [.tmp/pa056-review-prompt-3.txt](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-review-prompt-3.txt)
