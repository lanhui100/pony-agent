# Wave-43 NFR 达标报告（Agent Trace → OTel/Langfuse）

> 基准：`.dev-team/nfr-baseline-wave43.json`（`dev-nfr-lint.py` Exit 0）
> 收据目录：`.dev-team/evidence/wave43/`｜评审：`review-wave43-{a,b,l3p}.json`

| # | 基准字段 | 实现证据 | 结论 |
| :--- | :--- | :--- | :--- |
| 1 | `external_call_timeout_ms=3000` | `agent/otel/mod.rs` `EXPORT_TIMEOUT_MS=3_000` → `.with_timeout(...)`；`force_flush` 独立线程 + `recv_timeout`，`wait_ms = min(timeout_ms, 5000)`；L2-AT 黑洞用例实测落点 `[2000, 3250]ms` | **PASS** |
| 2 | `retry.max_attempts=1` | 导出失败不重试；L2-AT 断言黑洞收到的连接数 ≤2 | **PASS** |
| 3 | `logging.format=json` + `trace_id_required` | `agent/otel/log.rs::log_json`：单行 JSON，键 `level`/`message`/`ts`/`trace_id`（None → `null` 不省键）；调用点 level 全为字面量（AT-1b 静态断言）；新路径无裸 `eprintln!`（F4 已修） | **PASS** |
| 4 | `concurrency_lock.reporting_gate` | `AtomicBool`（无 mutex）；AT-3 Barrier 3×200 翻转 5s 内无 panic/无死锁 | **PASS** |
| 5 | `concurrency_lock.turn_scope = per_turn_closure` | 父 span 按 `tokio::task::Id` 索引 + `thread_local` 兜底（F5/A9 修复后）；守卫 Drop 精确摘除；并发 turn 单测父子关系正确 | **PASS** |
| 6 | `data_egress`（关闭即零出口） | `GatedSampler` 关闭态 `Drop`（零 span 分配）；导出器逐 span 过滤（F3/F6 修复后**仅**放行 `pony.otel.forced`）；wire 级单测断言关闭态上传只出被重放 session | **PASS** |
| 7 | `resource_release` | `RunEvent::Exit → otel::shutdown()`（幂等可重入、可再 init）；先 `drop(provider)` 再 `drop(state)`（A7 修复）；`BatchConfig` 显式有界 `max_queue_size=2048` / `max_export_batch_size=512` / `scheduled_delay=5s`（A5） | **PASS** |
| 8 | 敏感数据出境 | `OtelConfig` 手写 `Debug`（header 恒 `[REDACTED]`）、`config()` 剥离敏感 header、`RuntimeExporter` Debug 不转发上游（F1）；所有字符串属性过 `ponysentry::sanitize`（F2）；「http + 令牌 + 非回环主机」`init` 直接 Err（A3） | **PASS** |
| 9 | 配置模板与自检 | `.env.example` 含 4 个 OTEL 变量 + 用途/缺省/占位；`node scripts/check-secrets.mjs --all` Exit 0；`tests/env-example.spec.ts` 6 passed | **PASS** |
| 10 | 契约兼容性 | 破坏性删除（`AgentTracePayload` 等 13 符号）在 ADR 0047 §6 附三项兼容性证据；OT-7 全仓 0 命中强断言（21 passed）；保留面双向断言（issue/panic 面未损） | **PASS** |

## 机器收据汇总

| 命令 | Exit | 结果 |
| :--- | :--- | :--- |
| `cargo check -p pony-agent-core --target-dir target-check` | 0 | 仅既有 2 warning |
| `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check` | 0 | 0 error |
| `cargo test -p pony-agent-core --lib` | 101 | 1089 passed / **1 failed（HEAD 既有基线缺陷**：`agent::config::tests::missing_capabilities_fields_use_stable_defaults`，`config.rs` 本波零改动） |
| `cargo test -p pony-agent-core --test otel_trace_migration_test` | 0 | 21 passed |
| `cargo test -p pony-agent-core --test otel_trace_adversarial` | 0 | 6 passed |
| `cargo test -p pony-agent-core --test ponysentry_issue_resilience_test` | 0 | 2 passed |
| `npx vitest run` | 0 | 93 files / 986 passed / 10 skipped |
| `npx vue-tsc --noEmit` | 0 | 无 diagnostics |
| `npx vitest run tests/env-example.spec.ts` | 0 | 6 passed |
| `node scripts/check-secrets.mjs --all` | 0 | 0 leaks |

## 遗留与后续

1. **公网 OTLP 入口未实施**（用户裁决：新会话处理），交接单 `.dev-team/handoff-otel-public-ingress.md`。
2. **基线红测**：`missing_capabilities_fields_use_stable_defaults` 为 HEAD 既有缺陷（常量 `1_000_000` vs 断言 `256_000`），建议单列 backlog 裁决产品语义。
3. **可观测性巡检**：`tracing` 由 0/5 升至 4/5；`T3` 为巡检器启发式未识别 `Tracer::build_with_context` 的假阴性；`M3`/`L3`/`L4` 属 infra 侧（scrape job、log-collector 白名单），随公网入口同批处理。
