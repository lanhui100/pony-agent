# Wave-2 契约矩阵变更登记：Token 速度口径与上下文用量诊断

> 关联：`.dev-team/nfr-baseline.json`、`tests/unit/token-speed-metrics.spec.ts`（红相锚定 f16465c）、
> L3-P 审查 `.dev-team/review-l3p-wave2.json`（ADVISORY：wave-15 §S3 与本轮口径冲突需登记）。

## 1. 变更类型

| 条目 | 类型 | 说明 |
| :--- | :--- | :--- |
| Wave-15 §S3「严禁用整轮累加 output_tokens / 整轮 elapsed duration 计算速度」 | **breaking（有界收窄）** | 仅对**多调用轮（`providerCallRecords.length > 1`）**继续生效；**单调用终态轮**允许改用整轮聚合口径。 |
| 新增 `TokenSpeedCalculationMode` 四态 | additive | `call_active_stream` / `call_fallback` / `turn_completed_aggregate` / `none`。 |
| 新增 `resolveContextUsageMetrics` / `resolveContextUsageDiagnostics` / `formatTokenSpeed` | additive | 纯函数，无副作用。 |
| 新增 `metrics-diagnostics.ts`（`recordMetricsSnapshot` / `reportMetricsAnomaly`） | additive | 唯一埋点落点；签名去重 + 面包屑 8 条限幅 + 异常 60s 节流。 |
| `HomeStatusPanel` 新增可选 prop `tokenSpeedCalculationMode` / `speedDebugInfo` | additive | 向后兼容（缺省不渲染调试载荷）。 |
| OTel span 新增属性 | additive | `gen_ai.speed.tokens_per_second`、`gen_ai.usage.context_window`、`gen_ai.usage.context_ratio`、`agent.turn.speed.tokens_per_second`。无字段删除/重编号。 |

**删除 / 重编号：无。** 故不涉及 `reserved` 保留与消费方普查。

## 2. 冻结解析序（终态轮，唯一真值）

```
terminal(turn):
  records = turn.providerCallRecords ?? []
  if records.length > 1:
      → 末条有效 record 速度 (output/(duration−ttft));  mode = call_fallback
  else if records.length == 1:
      → if turn.outputTokens != null && turn.turnDurationMs > 0:
             turn 级聚合 outputTokens/(turnDurationMs/1000);  mode = turn_completed_aggregate
        else: 该 record 速度;  mode = call_fallback
  else:  # records.length == 0
      → if traceTimeline 有 canonical call_model 且 (firstTokenLatencyMs != null || outputTokens != null):
             该条目速度 (TTFT 抵扣);  mode = call_fallback
        else if turn 级数据有效: turn 级聚合;  mode = turn_completed_aggregate
        else: null;  mode = none
```

活跃轮（`isSubmitting` 且为最新轮）：优先末条 record 的 TTFT 抵扣单次速度，`mode = call_active_stream`。

**兼容性证据来源**：
1. `tests/context-display-tokens.spec.ts:168-208`（终态 records=0 + timeline 带 TTFT → 仍为 100 t/s）→ 未破坏；
2. `tests/HomeSidebar.spec.ts:511`（速度 Tooltip 文案精确 `"生成速度"`）→ 未破坏（模式改走 `data-speed-mode`）；
3. `tests/unit/token-speed-metrics.spec.ts`（单调用终态聚合 100 / 无 records 聚合 128）→ 满足；
4. 真实生产路径可达性：`src/stores/runtime.ts:4013` 终态写入 `providerCallRecords`（wire 全量 per-call 记录）+ `:3905/:4030` turnDuration 补丁 ⇒ `records.length == 1` 的终态轮确实命中聚合分支；
5. 真实浏览器收据：`.dev-team/evidence/wave-2-metrics/completed.png`（250 t/s）与 `active.png`（267 t/s）。

## 3. 调试诊断通道与 OTel 属性状态

| 通道 / 属性 | 状态 | 门控 / 来源 | 内容与公式 |
| :--- | :--- | :--- | :--- |
| `?debug=1` / localStorage `pony-agent.debug.runtime-logs` | active | `isDebugLoggingEnabled()` | `debugLog("metrics:token-speed", {...})` 结构化 JSON（含 turnId、mode、speed、in/out tokens、dur、ttft、window、ratio） |
| `data-speed-mode` | active | always | 当前速度口径（四态之一），DevTools 直接可读 |
| `data-speed-debug` / `data-context-debug` | active | **仅调试开启时渲染** | `in=..;out=..;dur=..;ttft=..;win=..;ratio=..` / `in=..;win=..;pct=..;mode=..;speed=..`，值与速度**同源** |
| PonySentry 异常面 | active | 60s/同 code 节流 | `invalid_duration` / `negative_speed` / `context_overflow`，`tags.category=metrics_governance` |
| `gen_ai.speed.tokens_per_second` | **active（已接线）** | `llm.chat` span | 接线点：`crates/pony-agent-core/src/agent/turn_flow.rs`（`provider_decision`:2278, `provider_decision_stream`:2319, `provider_followup`:2357, `provider_followup_stream`:2407）。口径：`output_tokens / ((elapsed_ms - ttft_ms) / 1000)`（流式可信 TTFT）或 `output_tokens / (elapsed_ms / 1000)`（缓冲式/退化），守卫与前端完全对齐。 |
| `gen_ai.usage.context_window` | **reserved（未接线）** | `llm.chat` span | `ProviderRequest` 目前无 context window 字段，agent 层无直接模型窗口元数据，留待后续波次从 settings/model registry 注入。 |
| `gen_ai.usage.context_ratio` | **reserved（未接线）** | `llm.chat` span | 依赖 `context_window`，同留待后续波次。 |
| `agent.turn.speed.tokens_per_second` | **reserved（未接线）** | `agent.turn` span | 需在 turn 终态汇聚处统计整轮耗时与聚合 token，留待后续波次接线。 |

## 4. 已知 ADVISORY 与治理登记

1. `debugLog` 未携带 OTel `trace_id`（前端侧无该上下文）；已在 payload 内带 `turnId` 作关联键，后续可由 Rust 侧回填。
2. 多调用轮终态仍显示「末次单调用速度」（沿用 Wave-15 §S3 冻结），未扩展为多调用加权口径。
3. L3-P 审查 `.dev-team/review-l3p-wave2.json` ADVISORY 项已闭环处理：
   - Wave-15 §S3 与本轮口径冲突已在 §1/§2 明确收敛定义；
   - `gen_ai.speed.tokens_per_second` 已在 `turn_flow.rs` 真实接线落地，其余未接线 OTel 属性已诚实标注为 reserved。
