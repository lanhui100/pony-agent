# B060 契约矩阵（Lead 冻结，wave-b060）

主题：Provider SSE 流式 tool call 缺 name 的诊断日志降噪 + 修复链路钉死。
分级：A 级（单生产文件 + 单测试文件；无新接口、无新网络 IO、无序列化契约变更）。

| ID | 断言（机器可判） | 承载测试 | 状态 |
| --- | --- | --- | --- |
| C1 | 同流同 index：`observe(index)` 仅首次返回 `true`，其后恒 `false` | openai_sse_diag_tests | frozen |
| C2 | 同流不同 index 互不吞噬：index 0 与 index 3 各自首次均返回 `true` | openai_sse_diag_tests | frozen |
| C3 | `suppressed_total()` 精确等于被静默的重复观测次数 | openai_sse_diag_tests | frozen |
| C4 | 无观测 → `take_summary()==None`；有抑制 → `Some((suppressed_total, 受抑制 index 数))` 且只返回一次（二次调用 `None`）；有观测但零抑制 → `None` | openai_sse_diag_tests | frozen |
| C5 | 空名 + `{"calls":[…非空…]}` 经 `normalize_tool_directive` 修复为 `BatchExecute`，不报错、不丢弃 | runtime/tests.rs | frozen |
| C6 | 不变量零变更：空名+非空参数仍走 `infer_tool_name_from_arguments` 修复；空名+空参数仍降级为纯文本；诊断行前缀 `provider:sse-tool-call-missing-name` 与 `preview_text(_, 400)` 上限不变 | 既有测试 + grep 复核 | frozen |

非目标（Non-Goals）：不改参数推断规则；不改 SSE 解析/合并语义；不引入日志级别框架；不改 `provider_log` 实现；不改 `runtime/stream_support` 修复逻辑。

写域：Executor = `crates/pony-agent-core/src/agent/provider/openai_sse.rs`；Test Agent = `crates/pony-agent-core/src/agent/provider/openai_sse_diag_tests.rs` + `crates/pony-agent-core/src/agent/runtime/tests.rs`。
