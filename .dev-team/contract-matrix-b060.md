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

## v1.1 修订（R2 打回后，Lead 裁决）

触发：L3-B 审查 BLOCKING（汇总行前缀复用巡检针）+ ADVISORY（测试零断言日志文本，接线回归仍全绿）。

| ID | 断言（新增/修订） | 状态 |
| --- | --- | --- |
| C1r | `diagnostic_line(index, item_preview)`：首见返回 `Some(line)`，`line` 以 `provider:sse-tool-call-missing-name index=` 开头、含 `item_preview=`、**逐字保留**传入的 preview；重复观测返回 `None` 且 suppressed 计数 +1 | frozen |
| C4r | `summary_line()`：无抑制 → `None`；有抑制 → `Some(line)`，含 `suppressed_total=` 与 `suppressed_indexes=`，且只返回一次（二次 None） | frozen |
| C7 | 汇总行文本**不得包含** `sse-tool-call-missing-name` 子串（B060 自身巡检针口径隔离：改完后 `grep -c 'sse-tool-call-missing-name'` 只应命中诊断行，汇总行用 `sse-missing-name-suppressed`） | frozen |

API 演进：`observe`/`take_summary` 由 `diagnostic_line`/`summary_line` 取代（后者返回可断言的日志文本，使接线本身可被机器测试钉死）；`suppressed_total()` 保留。

非目标（Non-Goals）：不改参数推断规则；不改 SSE 解析/合并语义；不引入日志级别框架；不改 `provider_log` 实现；不改 `runtime/stream_support` 修复逻辑。

写域：Executor = `crates/pony-agent-core/src/agent/provider/openai_sse.rs`；Test Agent = `crates/pony-agent-core/src/agent/provider/openai_sse_diag_tests.rs` + `crates/pony-agent-core/src/agent/runtime/tests.rs`。
