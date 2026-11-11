# ADR 0086：provider 上游失败分类补齐——边缘 5xx 与传输层标志纳入可重试

Status: implemented

日期：2026-11-07
契约真值源：`crates/pony-agent-core/src/agent/retry.rs` 的
`UPSTREAM_SERVER_ERROR_HINTS` / `UPSTREAM_EDGE_SERVER_ERROR_HINTS` / `UPSTREAM_TRANSPORT_HINTS` /
`upstream_code_for_hints` / `ProviderRetryPolicy::classify` / `hints_match`

## 背景

TUI 长跑出现文案「上游服务暂时不可用，正在按既有预算重试。」（ADR 0070 的
`ERROR_SUMMARY_TRANSIENT`），但日志显示绝大多数失败**一次都没有重试**：

| 观测量（`~/.pony-agent/tui-logs/tui-runtime.log`，147MB） | 数值 |
| :-- | --: |
| `attempt=1/5` / `attempt=2/5` | 99 / 2 |
| `decision=Abort { reason: NonRetryable }` | 85 |
| 上游状态码 429 / 400 / **522** / 401 / **525** / 503 | 69 / 54 / **36** / 18 / **12** / 2 |
| 传输层失败（`type=connect(+request)` 或 `type=request`，无状态码） | 144 |

即 5 次预算 + 指数退避在 `provider_timeout_backoff_config()`
（500→1000→2000→4000ms，cap 8s，预算 30s）里存在，却因**分类器直接 Abort** 而从未启动。

两类失败落在码表盲区：

1. **边缘 5xx**：`522 Connection Timed Out`、`525 SSL Handshake Failed` 等非 RFC 标准码，
   reqwest 显示为 `<unknown status code>`；③ 组只有 `502/503/504/408` ⇒ 不命中 ⇒ `None` ⇒ `NonRetryable`。
2. **传输层失败**：`error sending request for url (...)；type=request；elapsed=30996ms；source=client error (SendRequest)`。
   报文里**没有任何状态码**，④ 组只认 `connection reset` / `connection refused` / `dns` 三种措辞，
   而 reqwest 的 `SendRequest` / `Connect` 原文不含这些词 ⇒ 不命中 ⇒ `Abort`。

## 候选方案与拒绝理由

- **A. 5xx 全量放开（`5xx` 通配或 500–599 列表）**：拒绝——与既有冻结直接冲突
  （`stage2_upstream_code_for_hints_covers_r1_groups` 锁定 `provider returned 500` → `None`），
  且 501/505/508/511 属永久性协议错误，重试只会拖长失败感知。
- **B. 在 ③ 组内按语义补齐边缘码，另立 ④′ 组承接无状态码的传输失败**：采用。
- **C. 把 `connect` / `request` 作为裸词加进网络组**：拒绝——R1④ 明确禁扩 `connect` 泛词
  （单测锁定 `upstream_code_for_hints("connect") == None`），且裸 `request` 会命中
  任何「请求」措辞，判据会退化回整段文本 `contains`（PA-143 已判为缺陷）。
- **D. 传输标志判据前移到网络组之前**：拒绝——报文同时含真实 `400` 时会先命中传输标志，
  把「服务端明确拒绝」误升为可重试，纯属烧预算。
- **E. 顺带引入 jitter 随机源（`JitterKind::Equal`）**：本轮不做，理由见「影响·不改的部分」。

## 决策

1. **新增 ③′ `UPSTREAM_EDGE_SERVER_ERROR_HINTS = ["520","521","522","523","524","527","530"]`**，
   映射 `ErrorCode::UpstreamServerError`（`TransientRetryable`）。语义统一为「边缘没能与源站完成交换」，
   与既有 502/503/504 同组同结论。
2. **525/526 明确排除**：源站 TLS 握手失败 / 证书无效属配置错误，重试不会改变结果，
   维持 `NonRetryable → Abort`（不为让退避「看起来有效」而吞掉配置类故障）。
3. **新增 ④′ `UPSTREAM_TRANSPORT_HINTS = ["type=connect","type=request"]`**，映射
   `ErrorCode::UpstreamNetwork`；判据放在 `upstream_code_for_hints` 的**所有**状态码/措辞判据之后，
   保证状态码优先。
4. **词边界判据不变**：沿用 `hints_match`（数字码前后不得紧邻字母数字），
   因此 `elapsed=52201ms` / `token id 1522` 不会误命中；`type=decode`（SSE 解析失败）也不在集合内。
5. **不改动的既有语义**：429 / 408 / 500 / 400 / 401 / 403 / context-overflow 分组、
   `RATE_LIMIT_LONG_BACKOFF_HINTS` 与 `is_rate_limit_error` 七词、`decide` 的流安全闸门
   （有可见输出或已提交工具调用仍禁止自动重试）全部冻结不动。

## 影响

- **行为面**：522 类（实测 36 次）与传输类（实测 144 次）由「一次都不重试」变为进入既有
  5 次预算与指数退避；`Abort { NonRetryable }` 占比相应下降。429/400/401/525 的结论不变。
- **代价（明确接受）**：真·长期不可用场景下，用户看到终态提示的时间由「一次超时（约 31s）」
  推迟到「最多约 30s 预算 + 若干次尝试」。这是**既有预算配置的既定行为**，本次未改预算与退避参数。
- **不改的部分（另案，本 ADR 不覆盖）**：
  - `JitterKind::None` 保持。采样硬编码 `0.5` 使 `Equal` 抖动退化为恒定 ×0.75，真抖动需引入随机源
    并改写既有 `slept_ms` 断言；单机桌面客户端不存在同时重打的惊群面，收益不足。
  - `SingleHopRetryReport`（attempts/slept/exhausted_reason）仍只写日志、未进 `UiEvent`，
    TUI 因此仍拿不到真实 `n/N` 进度（ADR 0070 相关缺口的另一半）。
  - `continue_with_tool_results`（N>1 批量回填）仍无 `retry_provider_followup_timeout` 包装。
- **§3 观测性**：四维归属不变（tracing / issues 仍 applicable，落点不变；metrics / logs 结构性 N-A），
  本次只改**失败分类判据**，不新增对外接口、网络 IO、后台任务或 LLM 调用，不新增收口点。
  登记见 `docs/observability.yaml` 的 `wave-provider-transport-classification` 条目。

## 验收

| 契约 | 断言 | 用例 |
| :-- | :-- | :-- |
| C1 边缘 5xx（520–524/527/530）→ `UpstreamServerError` | 带 `<unknown status code>` 真实报文逐码命中 | `agent::retry::tests::edge_5xx_maps_to_server_error_but_origin_tls_stays_abort` |
| C2 525/526 仍 `None`（Abort），500 冻结不变，长数字内不误命中 | 同上 | 同上 |
| C3 传输标志 → `UpstreamNetwork`；裸 `connect` 仍不命中 | `type=connect+request` / `type=request` 真实报文 | `agent::retry::tests::transport_flags_map_to_network_after_status_code_judgement` |
| C4 状态码优先于传输标志；`type=decode` 不升格 | `400 + type=request` 仍 `UpstreamBadRequest` | 同上 |
| C5 `classify` 可重试性回归（正 3 / 负 3） | 522/520/传输类可重试，525/400/500 不可 | `agent::retry::tests::adr_0086_regression_classify_retryability` |
| C6 R1 八分组逐项不回归 | 既有码表断言全绿 | `agent::retry::tests::stage2_upstream_code_for_hints_covers_r1_groups` |
