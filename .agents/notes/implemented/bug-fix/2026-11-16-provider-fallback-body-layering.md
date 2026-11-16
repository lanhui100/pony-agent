# Agent Note: provider 本地兜底正文与诊断细节分层——用户只读一行，完整 error 只进 trace

Status: implemented

## Problem

现场会话的降级轮把 `local_tool_followup_fallback_response`（`crates/pony-agent-core/src/agent/provider/mod.rs:5999-6027`）的正文原样写进了 assistant 消息：

```text
工具 `Write` 已执行完成，但 provider 在整合工具结果时失败。
失败原因：error sending request for url (https://tokens.ponyjob.top/v1/chat/completions)；type=timeout+request；elapsed=21161ms；…
本地兜底摘要：…
结果预览：<工具输出前 2200 字>
```

三个后果：

1. **用户不可读**：URL、`elapsed=`、请求类型等技术串对用户没有意义；
2. **污染历史与后续上下文**：正文经 `assistant_message`（6021）进 native transcript、`turn:completed.text`、`trace.output_text`，也会进下一轮 provider 上下文与工具剪枝判定（大段结果预览占用预算）；
3. **诊断信息错位**：真正需要完整 error 的地方是 trace/provider 日志（`fallback_reason` 已是载体），而不是会话正文。

## Decision

把「用户文案」与「诊断细节」分层：

1. 新增 `pub fn fallback_body_text(tool_name: &str, error: &str, summary: &str) -> String`（provider 模块）：
   - **单行**，不换行；
   - 内容 = 工具名 + 「已执行完成，但上游整合结果失败」 + 原因**类别** + 一句可执行建议；
   - 原因类别由 error 文本推断：`timeout/timed out` ⇒ 超时；`connect/dns/network` ⇒ 连接失败；`decode/body` ⇒ 响应解析失败；其余 ⇒ 上游返回异常；
   - **禁止**出现 `http://`、`https://`、`elapsed=`、原始 URL、provider source、堆栈等技术串。
2. `local_tool_followup_fallback_response`：`output_text` 只取 `fallback_body_text(...)`；完整 `error` 仅写 `fallback_reason`（进 trace/provider 日志），不再进正文/`assistant_message`/`trace.output_text`。
3. 结果预览（原 2200 字）不再进正文；可执行信息由 `local_tool_followup_summary` 的短摘要承担（≤120 字）。

## Alternatives considered

- **A. 只在正文里截断 error 到 80 字**：落选。截断后仍会泄漏 URL 与 `elapsed=` 片段，且把「诊断」与「用户文案」继续混在一个字段里。
- **B. 正文完全不提失败，只给「已完成」**：落选。用户会误以为模型正常作答；降级必须可见（与 `turn_end.reason=degraded` 配套）。
- **C. 忽略原因类别，统一「上游异常」**：落选。超时/连接/解析三类对用户决策不同（重试 vs 换网 vs 缩减上下文），类别是可廉价获得的真实信息。
- **D. 同时改 `runtime/tool_recovery.rs` 的同族构造器**：本轮不做。它同属降级正文族但走另一条恢复路径，需单独验证影响面；已登记为后续项，避免把两个注入点混在一次变更里。
- **E. 把完整 error 写进 assistant 的 metadata 字段**：落选。会话历史已由 trace 承载诊断，正文/元数据再存一份就是双写；`fallback_reason` 是既有唯一真源。

## Consequences

- 用户侧降级轮只看到一行「工具 X 已执行完成，但上游整合失败（超时）。已保留执行结果，可继续或稍后重试。」——可读、可决策。
- 完整 error 仍可在 trace（`fallbackReason`）与 provider 日志中取证，`turn_end.reason=degraded`（ADR 2026-11-16-degraded-turn-reason-and-trace）与检查点投影让事实不丢。
- 后续上下文不再被 2200 字结果预览挤占，降低下一跳预算压力。
- 无新增对外接口/网络 IO/后台任务/LLM 调用；不新增遥测出口。
- `runtime/tool_recovery.rs` 的同族降级正文未改，属已知残余面（同族不同路径），需要后续单独收口。
