# Agent Note: OTLP 导出失败要分类，且失败摘要落有界本地 spool

Status: implemented

## Problem

`tui-runtime.log`（12:20–12:59Z）持续出现：

```
OTLP 导出第 1/2 次尝试失败…source: TimedOut  (https://otel.ponyjob.top/v1/traces)
OTLP 导出失败（2/2 次尝试后放弃，本批 N 个 span 永久丢失）
```

同窗口 provider 也在超时——同一个出口网络质量问题。后果不止「少几根线」：

- **告警诊断链自身缺证据**：`TraceSpanError` 类告警无法回溯到具体 span，用户收到的告警变成「无法自证」；
- **无法区分原因**：日志一律「导出失败」，不区分「出口网络不可达」与「鉴权/配置错误」，排查方向完全相反；
- 失败即丢，事后（尤其跨天）没有任何本地痕迹。

## Decision

1. **先分类再措辞**：`agent::otel::spool::classify_export_error` 纯函数把错误文本分四类
   （`Network` 出口网络 / `Auth` 鉴权 / `Config` 配置端点 / `Other`），判定顺序
   「网络 → 鉴权 → 配置」，因为 `TimedOut` 常与 `connect` 同现，先认网络才不会把用户引向错误方向。
   日志按类别给出人读短语（`出口网络不可达/超时` 等）。
2. **失败批次落有界本地 spool**：`otel-spool/export-failures.jsonl`（与 `sessions.db` 同级，
   复用 `sqlite_session::default_sqlite_path` 的数据目录解析，含 `PONY_AGENT_DATA_DIR` 旁路与测试隔离）。
   每条记录是一行扁平 JSON：`{ts_ms, kind, kind_label, spans, error(截断 240 字)}`。
3. **有界性双闸**：行数上限 200 + TTL 24h，写前剪枝（`prune_records` 纯函数）；
   写盘用 `tmp + rename` 原子替换，坏行（无 `ts_ms`）一律剪掉。
4. **不做 OTLP 重放**：`SpanData → protobuf ResourceSpans` 的 transform 是 `opentelemetry-otlp`
   私有实现，重放等于自维护一份协议编码；本轮只保证「丢了什么/为什么丢」可查，重放留作独立需求。

## Alternatives considered

- **加大导出超时/重试次数**：黑洞端点下只是把线程阻塞时间拉长（导出线程独占，队列同时堆积），
  与「可观测性不得拖慢主流程」的既有取舍冲突。否决。
- **自建 protobuf 重放（把 span 序列化后后台重发）**：需要复刻 SDK 的 transform 与 Resource 分组，
  版本升级即漂移，收益（回填历史 span）远小于维护成本。否决。
- **只在内存里留最近 N 条失败**：进程重启即丢，跨天排查无能为力——正是本次现场要解决的场景。否决。
- **失败后直接 `eprintln!` 完整 body**：会把长响应体灌进日志面，且没有聚合口径（无法按类别统计）。否决。

## Consequences

- 导出失败现在回答三个问题：类别、批次规模、错误摘要；跨天可查（24h TTL）。
- spool 只保留**摘要**，不是可重放的 span——不要据此声称「span 没丢」；它保的是诊断线索。
- spool 写失败只留一行 warn，绝不影响导出/主流程（可观测性自身不得再制造故障）。
- 回归口径：`cargo test -p pony-agent-core --lib otel::spool`（分类、截断、TTL/上限剪枝、坏行丢弃、端到端追加）。

## Related

- `2026-11-15-otel-span-sampling-and-nfr.md`（导出预算与 NFR 口径）
- B202 原始取证：provider 与 OTLP 在同一时段共同超时 ⇒ 出口网络问题优先于采集端配置
