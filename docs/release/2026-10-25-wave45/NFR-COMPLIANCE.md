# Wave-45 NFR 达标报告（可观测性语义收敛：PA-153 / PA-154 / PA-155）

> 基准：`.dev-team/nfr-baseline-wave45.json`（`dev-nfr-lint.py` Exit 0）
> 契约：`.dev-team/contract-matrix-wave45.md`｜红相：`.dev-team/wave45-red-receipt.log`
> 评审：`review-plan-wave45-{a,b}.json`、`review-wave45-{a,b}.json`、`review-l3p-wave45.json`
> 收据：`.dev-team/evidence/wave45/`

| # | 基准字段 | 实现/测试证据 | 结论 |
| :--- | :--- | :--- | :--- |
| 1 | `pa_153_resource_attribution`：三键齐全、版本默认 core crate、env 覆盖、空白回落 | `otel/mod.rs` `resolve_resource_attributes` + `init` Resource；PA153-T1/T2 + 3 个实现期单测 | **PASS** |
| 2 | `pa_154_turn_error_aggregation`：根 ERROR、`turn_error: ` 前缀、outcome=error、全成功无键、phase=failed 优先 | `TurnTraceGuard::drop` + `mark/take_active_turn_error`；PA154-T1/T2/T3/ADV | **PASS** |
| 3 | `pa_155_local_cache_timeout`：15000ms 有界、缓存保留、迟到无写、host 读 1 次、watchdog 不变 | `runtime.ts:2678-2686`；PA155-T1/T2 | **PASS** |
| 4 | `external_call_timeout_ms=3000` | 零新增网络/IO；PA-155 为本地 IPC 水合，N-A 理由见契约 §5 | **N-A** |
| 5 | `logging{format=json,trace_id_required}` | 无新增结构化日志面；`turn_error` 仅固定字面量 | **N-A** |
| 6 | `resource_release` | 聚合 Drop 取走即清零；`withTimeout` 结算清 timer；无新句柄 | **PASS** |
| 7 | `concurrency_lock` | task-id 表 + 线程本地栈双分支；先记录者胜；无锁重入风险 | **PASS** |
| 8 | 配置与自检 | `.env.example` 增 2 个可选 env；`docs/observability.yaml` 增 wave-45 归属登记 | **PASS** |
| 9 | 契约兼容 | `OtelConfig` 四字段与 7 处构造点零改动；新增 pub const additive | **PASS** |
| 10 | 版本对齐风险 | `service.version` 默认 core `0.1.128`，发布 `0.1.155`；宿主未设 env，契约 §1 risk + NFR `version_alignment_risk` 已声明 | **已知偏差（follow-up）** |

## 机器收据

| 命令 | Exit | 结果 |
| :--- | :--- | :--- |
| `cargo test -p pony-agent-core --test otel_resource_turn_status_test --test otel_resource_env_test --target-dir target-test` | 0 | 7 passed（status 6 + env 1） |
| `cargo test -p pony-agent-core --lib agent::otel --target-dir target-test` | 0 | 25 passed |
| `cargo check -p pony-agent-core --target-dir target-check` | 0 | 0 error |
| `cargo test -p pony-agent-core --lib --target-dir target-test` | 101 | 1154 passed / 2 failed：①既有 `config::missing_capabilities_fields_use_stable_defaults`；②既有 flake `ponysentry::governance_block_failure_is_not_reported`（模块级 20 次复跑 2 次失败，见 `evidence/wave45/flake-receipt.md`，非本波引入） |
| 同上 `-- --test-threads=1` | 101 | 1155 passed / 1 failed（仅既有 config） |
| `npx vitest run tests/acceptance/wave45` | 0 | 2 passed（红相：2 failed） |
| `npm run test:unit` | 0 | 103 files / 1036 passed / 10 skipped |
| `npm run build` | 0 | vue-tsc 0 + vite built in 5.29s |
| `check-declaration.sh` | 0 | 四维声明一致 |
| `audit-diff.sh --base f233a96 --decl-approved` | 1（WARN） | 仅「声明无收口点」既有 WARN，无违规 |

## 遗留（已登记）

- **service.version 双轨**：默认取 core crate 版本，发布版本需宿主设置 `PONY_OTEL_SERVICE_VERSION`（本波 Non-Goal）。
- **根 span 文案不含子错误明细**（仅 `turn_error: <reason>`）：告警下钻需回连子 span（advisory）。
- **两个新 env 未过 sanitize**：来源为进程 env 而非用户内容（advisory）。
- **PA-155 既有语义**：真实 host 错误仍被 `.catch(()=>null)` 降级为缓存视图（与 HEAD 一致，非本波引入）；迟到 resolve 闭包驻留。
- **watchdog 竞态**：3500ms 解锁与 15000ms 水合之间仍可能迟到写回（与 wave-44 一致，登记 backlog）。
- **既有 ponysentry 测试 flake**：`turn_failure_capture_tests` 共享全局队列，并行下 ~10% 失败；建议 `serial_test` 或独立队列（backlog）。
