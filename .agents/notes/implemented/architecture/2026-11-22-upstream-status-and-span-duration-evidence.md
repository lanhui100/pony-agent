# Agent Note: 上游 HTTP 码结构化与 `llm.chat` span 时长排除重试等待（B139 / B135 决策）

Status: implemented

## Problem

两条 P2 债务同属「观测口径不可用于判定」：数据有，但读的人无法据此下结论。

**B139（`core::agent::provider::upstream_rate_limit_evidence`）**
上游测速/诊断需要 per-call 的 HTTP 响应码，用来区分「配额抖动期的 429」与「真实吞吐结论」。
实测样本：`gemini-3.8-flash-high` 12 次里有 8 次 `429 quota_exhausted`——这类样本不可用于速率比较，
但当前数据面**没有结构化状态码**可筛。

证据：provider 调用链的错误类型是 `String`，限流判定靠正文匹配
（`is_rate_limit_error(&err)`，`provider/mod.rs:5858`；测试 `provider/tests.rs:1994-2006` 全是对
`"HTTP 429 Too Many Requests"` / `"monthly quota exhausted"` 这类字符串的断言）。
`ProviderCallCacheRecord`（`agent/telemetry.rs:112`）也没有任何状态码字段。

**B135（`core::agent::otel::llm_chat_span_duration`）**
`llm.chat` span 的 duration 覆盖整个 guard 生命周期，因此包含**重试等待**（退避 sleep）。
一次 3 次尝试、退避 20s+40s 的调用，span 时长会被记成 60s+，与真实模型延迟无关。

证据：`ProviderCallCacheRecord` 已有 C5.1 落地的 `attempts` / `failed_attempt_ms` / `prelude_ms`
三个 additive 字段（`agent/telemetry.rs:150-163`），说明**账本侧口径已存在**；
但 span 侧没有读取它们做扣减，也没有把等待时长单列成属性。

## Decision

1. **B139 走「错误类型结构化」，不做正文正则加固。**
   在 provider 出口引入 `UpstreamStatus`（`Option<u16>`）随错误一并上抛，
   `is_rate_limit_error` 的判定顺序改为「有状态码 ⇒ 用状态码族；无状态码 ⇒ 保留现有正文匹配（兼容旧通道）」；
   `ProviderCallCacheRecord` 追加 additive 字段 `upstream_status: Option<u16>`
   （`skip_serializing_if = "Option::is_none"`，缺省键缺席，旧记录/旧消费者零影响）。
   拒绝让前端或诊断层自己从错误文案里 `parse::<u16>()`——那会把「文案格式」变成隐式协议。
2. **B135 走「span 属性单列 + duration 扣减」，并把口径写进 ADR 而不是散落注释。**
   span 收尾处按账本扣减：`duration_excluding_retry = duration - (failed_attempt_ms + prelude_ms)`
   （两者仅在真的重试时 `Some`），同时把 `llm.retry.wait_ms` 作为**独立属性**输出，
   使「真实模型延迟」与「等待」都可查，而不是只给一个混合数。
   无账本（`None`）时**不回退成 0**：属性缺省，duration 保持原值，避免伪造精度。
3. **两者拆成独立实施 PR，不与本决策 PR 混做**：
   - B139 需要改 provider 调用链的错误类型签名（跨多个 `Result<_, String>` 出口）与前端记录类型同步；
   - B135 需要改 otel span 收尾与 `ProviderCallCacheRecord` 读取路径；
   - 二者改动面不重叠，但都不小，且各自需要 Rust 单测锁定「状态码族优先级」与「扣减不出现负数」。
   在并发多 PR 的窗口里把它们塞进一个 PR 只会让失败归因变难。
4. **范围外**：不改 `usage_origin` / `context_input_tokens` 等既有 additive 字段语义；
   不为 `upstream_status` 造字符串枚举（保住「未知」与「无响应」的区别）。

## Alternatives considered

- **继续用正文正则（加固关键词表）**：零签名改动，但每次上游改文案就漏判；且 B139 明确要的是
  「可用于筛样本的结构化证据」，正则匹配出来的布尔无法区分 `429` / `503` / `quota` 三类。
- **只在诊断日志里打印状态码**：日志是本地取证面（有界环形缓冲），不进入 per-call 记录，
  无法在事后按样本筛「是否配额抖动」；等于把问题推给人工翻日志。
- **span duration 直接改成「只算最后一次尝试」**：能消除等待，但会丢掉「总耗时」这个真实运维指标；
  分解成 duration + wait_ms 两个数才能同时回答「模型多慢」与「用户等了多久」。
- **在 span 收尾处用 `saturating_sub` 静默夹到 0**：会在账本失真时伪造「零延迟」，
  比属性缺省更危险；故要求「无账本 ⇒ 属性缺席」。
