# Agent Note: SSE 缺名工具调用诊断日志降噪

Status: implemented

## Problem

`provider:sse-tool-call-missing-name` 是 provider 在上游 SSE 分片里「有 arguments 但缺 function.name」时打印的
排查诊断。上游 provider（如 ppx 类网关）常在整段 arguments 的**每个分片**都省略 name，导致同一条工具调用
被重复打印数十次，把真正有信息量的运行日志（工具调用、错误、审批）淹掉，也让日志本身失去可读性。

实测口径（2026-11-11 本机复跑，可复现）：
`grep -c 'sse-tool-call-missing-name' ~/.pony-agent/tui-logs/tui-runtime.log` → **1,705,040 条**，
该日志文件 **525,803,457 字节**（约 501 MB）。任务卡 `backlog:B060` 里的「1,633,467 条 / 近 30MB / 104,328 条」
是更早时点的历史快照，与当前实测非同一次采样——两处数字不可混用。

诊断本身**不能删**：缺名是本仓已确认的上游缺陷形态，删除等于放弃现场证据。缺的只是「同一条事实只该说一次」。

## Decision

在 `crates/pony-agent-core/src/agent/provider/openai_sse.rs` 引入 `MissingNameDiagnostics`（每个 SSE 流一个实例，
随累积器生命周期）：

- 同一 `index` 每流最多输出 1 条诊断行（首见即打印，字段与前缀逐字不变）；
- 重复观测只累计 `suppressed_total`，不再打印；
- 流结束（`finish`）时若存在被抑制的重复，输出**至多 1 条**汇总行
  `provider:sse-missing-name-suppressed suppressed_total=<N> suppressed_indexes=<M>`
  （前缀刻意避开巡检针 `sse-tool-call-missing-name`，见下「巡检口径隔离」）。

即日志条数上界从「∝ 分片数」压到「≤ 1 + 2×index 数」，同时保留「发生过多少次」的规模信息。

配套：`crates/pony-agent-core/src/agent/provider/openai_sse_diag_tests.rs`（C1r–C4r/C7 行文本语义）与
`crates/pony-agent-core/src/agent/runtime/tests.rs` 末尾的 C5 用例（空名 + `{"calls":[…]}` → `BatchExecute`）
钉死「空名仍能被 `normalize_tool_directive` 正确修复」的链路，防未来把「保留空名 tool call」的意图改掉。

## Alternatives considered

- **删掉这条诊断**：直接消灭刷屏，但也消灭了唯一的现场证据——下次上游改格式时无从取证。落选。
- **把 `provider_log` 降级为 debug 级并引入日志级别框架**：本仓 `provider_log` 是 `eprintln!` 直写，
  引入级别框架属于跨文件的基础设施改造，代价远超本次缺陷，且会牵动既有日志消费方。落选（若将来要做
  统一日志分级，应作为独立事项立项，而不是夹带在降噪里）。
- **按时间窗口采样（如每秒最多 1 条）**：仍有抖动、仍需状态，且不能回答「这一条工具调用到底漏了几次」，
  信息量不如「首条 + 汇总」。落选。
- **只在流结束打印一条统计**：实现最简单，但丢掉首条原文 preview，排查时看不到 provider 到底发了什么结构。落选。

## Consequences

- 诊断可读性恢复；规模信息以汇总行保留（`suppressed_total` / `suppressed_indexes`）。
- **巡检口径隔离**（审查 BLOCKING 后修订）：汇总行前缀为 `provider:sse-missing-name-suppressed`，**不含**
  `sse-tool-call-missing-name` 子串——否则 B060 自身的验收探针
  （`grep -c 'sse-tool-call-missing-name' ~/.pony-agent/tui-logs/tui-runtime.log`）会永远非 0、把汇总行误当诊断行。
- **接线可测（规格层）**（审查 ADVISORY 后修订）：降噪器不再只暴露布尔判定，而是构造日志文本
  （`diagnostic_line` / `summary_line`），使「前缀是什么、`item_preview` 是否逐字、汇总计数口径」可被单测断言。
  其结构性残留：`provider_log` 是 `eprintln!` 直写、无注入点，单测无法断言"这一行真的被打印"，
  故把调用点改成 `let _ = diagnostic_line(...)` 仍会全绿——该边界由代码审查而非机器门禁承担
  （引入日志注入点属跨文件基础设施改造，见备选条款，不在本次范围）。
- 契约 C6 不变量：修复链路（`infer_tool_name_from_arguments`）、空名+空参数降级纯文本、
  诊断前缀与 `preview_text(_, 400)` 上限均零变更。
- 已知限制（审查记录、本次接受不改）：
  1. 汇总行只在正常收尾（`finish`）打印；取消/超时/帧解析失败等异常流先经 `read_result?` 返回，抑制计数丢失——
     异常流本就有各自的错误日志，接受该信息损失。
  2. 同一 `index` 只保留首个分片的 `item_preview`；后续分片（常带真实 arguments）不再可见，仅计入抑制次数。
  3. `index` 字段缺失时沿用既有默认值 0（HEAD 同样如此，非本次引入），多个无名无 index 的调用会折叠统计。
  4. 被抑制的重复观测仍会先构造 `item.to_string()` + `preview_text` 再丢弃（`diagnostic_line` 入参在调用点求值）；
     HEAD 版本就每分片无条件构造，故非本次引入的回归，属"可再省一次的 CPU"而非行为缺陷，暂不改签名换惰性求值。
  5. `index` 规模无上界（上游可控字段）；`BTreeSet` 随不同 index 数增长，但条目数 ⊆ 既有 `tool_calls` 键集
     （≤3× 常数放大）且随流释放。若未来出现超大 index 攻击面需另立事项。

