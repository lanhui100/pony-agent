# Agent Note: 上游 HTTP 状态码入 per-call 记录的观察者实现（B139 落地）

Status: implemented

## Problem

B139 的决策（ADR `2026-11-22-upstream-status-and-span-duration-evidence`）要求「provider 出口引入
`UpstreamStatus(Option<u16>)` 随错误上抛」，目标是让 per-call 记录能筛掉配额抖动样本
（`gemini-3.8-flash-high` 12 次里 8 次 `429 quota_exhausted`）。

落地时的事实核查推翻了一个隐含假设：provider 调用链里 `Result<_, String>` **有 74 处**，
`ProviderCallCacheRecord` 的字面量构造点 **17 处**（跨 8 个文件）。按字面改签名意味着
74 个返回点全部换类型 + 17 个构造点全部补字段 + 所有 `.map_err(|e| e)` 的调用方适配；
一旦中间有遗漏，失败会以「记录字段缺失」而不是「状态码错」的形式暴露，归因成本远高于收益。

同时发现两个**单一收敛点**，使「不改签名」成为可行：
- 所有非 2xx 出口都经过 `provider::provider_status_error(status, …)`（4 处调用点共享）；
- 所有 per-call 记录都经 `build_provider_call_cache_record`（`runtime/stream_support.rs:310`）构建。

## Decision

1. **用线程局部「取走即清空」旁路传状态码**（`agent/telemetry.rs`）：
   `record_upstream_status(u16)` 写入、`take_upstream_status()` 读取并清空。
   这与 `agent::retry` 的 `RetryNotice` 观察者同一模式（同仓先例），不改任何错误类型。
2. **写入点唯一**：`provider_status_error` 内一行调用。**读取点唯一**：
   `build_provider_call_cache_record` 构造记录时取走。两点都有注释写明「唯一」，便于 review 核对。
3. **`ProviderCallCacheRecord` 追加 additive 字段 `upstream_status: Option<u16>`**：
   `skip_serializing_if = "Option::is_none"`，仅非 2xx 有值（**不写 200**）——「成功」与「没失败」
   在数据面上不需要重复表达，键缺席即未失败。
4. **重建/投影路径显式置 `None`**：`projection.rs` 的 trace 重建与其余 16 处构造点不携带状态码，
   一律写 `upstream_status: None` 并注明「此处无真实状态码 ⇒ 留空」，与既有
   `context_input_tokens: None` 的同款纪律一致（禁止臆造口径）。
5. **前端镜像同步一个可选字段**：`src/types/runtime.ts` 的 per-call 记录加
   `upstreamStatus?: number | null`；不新增任何消费逻辑（谁筛样本由后续面板决定）。

## Alternatives considered

- **按字面改 74 处签名（`Result<_, String>` → `Result<_, UpstreamError>`）**：类型最干净，但一次改动
  74 个返回点 + 17 个构造点，任何遗漏都表现为编译错误轰炸（本次大批量补字段时实测一次就出现
  `E0063`×17 与一处模式匹配误插），review 面与回归面都不成比例。
- **从错误正文里 `parse::<u16>()`**：正文格式是上游/我们自己的文案，不是协议；决策 ADR 已明确拒绝。
- **把状态码只写进日志**：日志是本地取证面，事后无法按样本筛「是否配额抖动」，等于把问题推给人工翻日志。
- **`#[serde(default)]` + 全部构造点写 `..Default::default()`**：会让「漏填」变成静默默认值，
  正好掩盖我们想暴露的信息（某条路径确实拿不到状态码）；显式写 `None` 更诚实。
- **跨线程共享（`static Mutex`）**：provider 调用与记录构建在同一 turn 线程上（与 retry 观察者同一前提），
  全局锁会把并行 turn 的状态码互相污染，比线程局部更不安全。
