# wave-metrics 交付验证收据（Lead 实测）

- 基线（红相锚定）：`5bb99639`
- 交付 commit：`abbe8231`（含 `[decl-change-approved]`）
- 环境：Linux x86_64 / rustc 1.91.1 / node v22.21.0

## 1. 机器门禁（全部真实 Exit 0）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 编译 | `cargo check -p pony-agent-core --all-targets` | Exit 0；warning 恒为基线 2 条（`mcp_channel_closure`），无新增 |
| 契约验收 | `cargo test --test metrics_snapshot_contract` | 10 passed / 0 failed |
| 对抗破坏 | `cargo test --test metrics_snapshot_adversarial` | 5 passed / 0 failed |
| TUI 消费 | `cargo test --test tui_metrics_snapshot` | 12 passed / 0 failed |
| 既有不回归 | `--test tui_token_context_wave41` / `tui_dialog_wave39` / `tui_streaming_input_acceptance` | 14 / 22 / 32 passed，0 failed |
| 契约快照 | `--test ts_contract_snapshot` | 1 passed / 0 failed |
| 前端类型 | `npm run typecheck`（vue-tsc --noEmit） | Exit 0 |
| 前端验收 | `npx vitest run tests/metrics-snapshot-live.spec.ts tests/acceptance/adversarial/metrics/malformed-payload.spec.ts tests/context-display-tokens.spec.ts` | 3 files / 17 tests passed |
| 契约漂移 | `npm run contract:check` | 4 行全 OK（含 `TurnMetricsSnapshot 22/22`、`TurnStreamEvent 38/38`） |
| 扩展点生成物 | `npm run ext:check` | PASS |
| 观测增量门禁 | `audit-diff.sh --base HEAD --decl-approved` | Exit 0（无越界实现、收口点完整） |
| 观测声明一致 | `check-declaration.sh` | Exit 0（metrics/tracing/logs/issues 四维 PASS） |
| 观测 ob2 物理门禁 | `npx vitest run tests/acceptance/wave47-ob2-enforcement.spec.ts` | 7 passed（T7 Exit 0） |

## 2. 红相 → 绿相

- 红相（基线 `5bb99639` 后、实现前）：`metrics_snapshot_contract` 1 passed / 8 failed（`unimplemented!`）、`adversarial` 0/5、`tui_metrics_snapshot` 编译失败（`apply_live_metrics`/`live_metrics` 缺失）、`vitest` 9/9 失败。
- 绿相（`abbe8231`）：上述全部转绿，且既有 3 套 TUI 回归零回归。

## 3. 审查收据（均 `dev-review-lint` Exit 0）

| 收据 | 结论 | 说明 |
| --- | --- | --- |
| `review-plan-wave-metrics.json` | FAIL（4 BLOCKING） | **补做**的方案级对抗审查；A7（去 clamp）/A8（轮级 Σ 成本）本波修复，R1/R2 登记 B030 |
| `review-join-wave-metrics-a.json` | FAIL（1 BLOCKING / 5 ADVISORY） | L3-A 并发/状态流转；goal 续轮快照泄漏 |
| `review-join-wave-metrics-a2.json` | PASS | L3-A2 独立复审（变异 + 双轮 reducer 新反例） |
| `review-join-wave-metrics-b.json` | PASS（0 BLOCKING / 7 ADVISORY） | L3-B 安全/协议一致性；I7 additive 成立、22 键对齐、无新出口 |
| `review-join-wave-metrics-l3p.json` | PASS（6 问全 PASS） | L3-P 六问基准映射否决制 |

## 4. 已知残余（已登记，不在本波修复）

- R1 首跳流式期无快照（回退 legacy 渲染）→ B030
- R2 快照不持久化（重放由 trace 反推）→ B030
- R3 compaction 无 basis → B028
- R4 缺席歧义（skip_serializing_if）→ B031
- R5 ratio 单位不自描述 → B031
- R6 ModelMonitor 第三计算面 → B027
- R7 事件体积无 CI 门禁 → B032

## 5. 环境性既有失败（与本波无关，已单跑隔离）

- `tests/check-secrets.spec.ts` 端到端零误报：命中 5 条既有夹具字串（`mcp_channel_closure.rs` / `plugin_host_contract.rs` / `.dev-team/l2at-done-*.json`），均非本波路径。
- `tests/update-signed-store.spec.ts` / `src/stores/__tests__/plan-goal-refresh.spec.ts`：全量并行下 flaky，单跑 12/4 全绿。
- `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts`：单跑 1 failed（既有）。

## 6. 工作区状态

- `git status --porcelain` 仅剩 2 个**本次之前既有**的未跟踪目录 `.dev-team/retro/pack-session-*`（他次会话产物，未删除）。
- 无后台进程/端口/临时服务残留。
