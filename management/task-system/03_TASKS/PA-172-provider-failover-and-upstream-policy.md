# PA-172: 跨 provider 自动故障转移 + 上游调用时限/重试配置化

## 状态

- **状态**: **实施中（wave-69）**，契约见 `.dev-team/contract-matrix-pa172.md`
- **优先级**: P1
- **Complexity**: B（跨 config / provider / retry / observability 多模块 + 外部网络调用面）
- **创建日期**: 2026-11-09
- **前置**: PA-171（wave-68，已合入主干；遗留 13 条见该卡 §遗留与后续）
- **决策依据**: ADR 0104（本卡同提交落盘）、ADR 0103（限流冷却与续轮止损）

## 需求背景（PA-171 遗留第 1 条升级为本卡）

PA-171 把"一次上游限流"收敛成"一次可见失败"，但**没有**解决"这一次失败能否换一条上游继续"以及
"45s 总超时 / 5 次重试 / 30s 退避预算为什么是硬编码"。诊断会话 `pa_01a1ba70-…` 的实际形态：

- 10:13–10:26：5 轮均在 `CALL MODEL` 阶段被 **45002ms** 总超时杀死（`provider/mod.rs:2737`/`:2788`
  两处内联 `Duration::from_secs(45)`，无常量、无 env、无配置通道）。
- 10:28:51 起同一 provider 转为 429 配额耗尽 ⇒ 该 provider 在 5 小时窗口内**完全不可用**，
  而仓内**没有任何跨 provider 换手能力**（仅同 provider 内的 followup 本地兜底：
  `provider/mod.rs:5092-5094`、`:1279-1317`、`tool_recovery.rs:471-475`；
  `RecoveryPipeline`（`recovery.rs:97-101`）生产未接线）。
- 结论：用户唯一出路是**手工换模型**（10:43 才恢复）。而"手工换模型"正是机器该做的事。

## 目标

1. 上游调用时限/重试次数/退避预算**可配置**，默认值与今天逐字一致（零行为变更）。
2. 单 provider 失败时可按**显式候选链**自动换手到下一个 provider 继续本轮（有界、可观测、不伪造成功）。
3. 换手与冷却协同：已冷却的候选**直接跳过（0 次 HTTP）**，不制造"冷却里再打一遍"的放大。

## 交付契约（语义真值源 = `.dev-team/contract-matrix-pa172.md`）

- **P1 配置面（additive）**：`providers.json` 顶层新增
  `upstreamPolicy { requestTimeoutMs, decisionMaxAttempts, retryBudgetMs }` 与
  `failoverProviderIds: [provider_id, …]`（有序候选链）。
  缺省 ⇒ 45000 / 5 / 30000 / 空链，行为与 PA-171 交付后完全一致。
- **P2 钳制（fail-safe）**：越界值**钳制到边界**并落 `warn` 日志 `upstream_policy_clamped`；
  非法/未知的 `failoverProviderIds` 条目忽略并 `warn`。禁止因手改配置让整个 provider 面失效。
- **P3 换手触发（封闭值域）**：仅 `classify_error_class(err) ∈ {rate_limited, quota, timeout, network, auth}`
  触发；`bad_request` / `other` **不换手**。
- **P4 换手边界**：最多 2 次换手（`MAX_FAILOVER_HOPS`）；跨 provider 总预算有硬上限；
  冷却中候选跳过；全链耗尽返回**最后一次错误原文 + 结构后缀**（可解析），禁止伪造成功。
- **P5 观测**：metrics 为结构性 N-A（`docs/observability.yaml:45-56`）⇒ 结构化 JSON 日志为唯一面，
  新增封闭事件 `provider_failover_switched` / `provider_failover_skipped_cooldown` /
  `provider_failover_exhausted` / `provider_cooldown_reset`（字段清单见矩阵 §6）。
- **P6 冻结不变**：`ProviderRetryPolicy::classify/decide` 语义、`RetryBudget` 判据、
  `ERROR_SUMMARY_TRANSIENT` 逐字文案、`emit_stream_failed` 14 参签名、
  `tests/pa171_*.rs` 四个冻结文件（只读）均不得改动。

## 非目标

- 跨 provider **健康度评分/自动发现**（候选链由用户显式配置，不做自动排序）。
- 流式 15s connect / 180s read-idle 的配置化（wave-69 只配置同步出口总超时与 RetryBudget；
  流式面登记后续）。
- 前端 provider 编辑 UI 的候选链编辑控件（本轮只保证配置 round-trip 不丢字段）。

## 波次与写域（wave-69）

| 任务 | 内容 | write_scopes |
| :-- | :-- | :-- |
| `protocol-agent` | 契约矩阵 + NFR 基准 + PA-171 遗留闭合表 | `.dev-team/` |
| `t-red` | 红相冻结（failover / config / lease / ts-contract） | `crates/pony-agent-core/tests/`、`docs/contracts/ts-contract.snapshot.json` |
| `t-w-provider` | P1–P5 实现 + 冷却遗留 #3/#4/#5/#6 | `agent/provider/`、`agent/config.rs`、`agent/retry.rs`、`bin/tui/errors.rs` |
| `t-w-core` | `errorClass` 前端类型 + 租约抽库 + TUI 遗留 #7~#13 | `bin/tui.rs`、`agent/control_plane/mod.rs`、`agent/goal.rs`、`agent/goal_lease.rs`、`src/types/runtime.ts` |

## 验收（物理收据）

1. `cargo check -p pony-agent-core --all-targets` Exit 0。
2. 新冻结测试全绿：`pa172_provider_failover_contract`、`pa172_upstream_policy_config_contract`、
   `pa171_lease_anchor_contract`（含租约分区/互斥/Drop 释放/双会话并发 + outcome 五臂×phase 五臂）。
3. 既有冻结测试零改动且绿：`pa171_goal_round_failure_contract`（7）、
   `pa171_provider_rate_limit_cooldown_contract`（5）、`pa171_failed_turn_attribution_contract`（3）、
   `pa171_error_copy_contract`（89）；`--lib` 全量；`tui_batch2_regression` / `tui_session_resilience_acceptance`。
4. `cargo test -p pony-agent-core --test ts_contract_snapshot` Exit 0 且快照含 `errorClass`；
   `node scripts/check-ts-contract.mjs` Exit 0（**未加前端字段时必须非零**，红绿方向已冻结）；
   `npm run typecheck`（vue-tsc）Exit 0。
5. `bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base <基线>` 与
   `check-declaration.sh <repo>` Exit 0。
6. Tier-1 冒烟：`cargo build -p ponycli`（或既有等价）Exit 0 + `--help` Exit 0。
7. 全部提交**定向暂存**（禁 `git add .`）；冻结测试文件 `git diff` 为空。

## 断点续跑指引

1. 读本卡 → `.dev-team/state-wave69.json`（本波账本）→ `.dev-team/contract-matrix-pa172.md`。
2. 语义真值源：本卡 §交付契约 + 矩阵（含 `wave-69-a1` amendment 对 PA-171 矩阵的追加）。
3. 唤起词：**「继续 PA-172」** / **「继续 PA-171」**（同一续跑入口）。

## 落地结果（物理收据，2026-11-09 / wave-69）

| 门禁 | 命令 | 结果 |
| :-- | :-- | :-- |
| 编译 | `cargo check -p pony-agent-core --all-targets --target-dir target-check` | Exit 0 |
| 新冻结验收（PA-172） | `--test pa172_provider_failover_contract` / `pa172_upstream_policy_config_contract` / `pa172_budget_anchor_contract` | 11 / 8 / 6 passed |
| 租约与归约锚点 | `--test pa171_lease_anchor_contract` | 6 passed |
| 遗留收口锚点 | `--test pa172_legacy_closure_contract` | 99 passed |
| PA-171 冻结面零改动 | 5 个 `tests/pa171_*.rs`（`git diff --exit-code`） | 未改动，89/89/3/7/5 全绿 |
| 单测 | `cargo test -p pony-agent-core --lib --target-dir target-test` | 1404 passed（与基线同数） |
| TUI 回归 | `tui_batch2_regression` / `tui_session_resilience_acceptance` | 96 / 98 passed |
| TS 契约 | `cargo test --test ts_contract_snapshot` + `node scripts/check-ts-contract.mjs` | 1 passed / Exit 0（`errorClass` 39/39 键） |
| 前端类型 | `npx vue-tsc --noEmit` | Exit 0 |
| 观测门禁 | `audit-diff.sh --base 98730d5d` / `check-declaration.sh` | Exit 0 / PASS(exit 0) |
| NFR 基准 | `dev-nfr-lint.py .dev-team/nfr-baseline-wave69.json` | PASS / Exit 0 |

**现场对照**：默认配置下行为与 PA-171 交付后逐字节一致（45s / 5 次 / 30s / 空链）；
显式配置 `failoverProviderIds` 后，`decide_with_tools` 一次失败可按链换手（hop ≤ 2、逐 attempt deadline 120s、
冷却候选 0 次 HTTP）；流式入口与 followup 入口**不换手**（见契约矩阵 §3.4 与 §11 偏差）。

**已知限制（如实登记，见契约矩阵 §11）**：流式无总 deadline ⇒ 不换手；followup 的 `accumulated_messages`
语义 ⇒ 不换手（deferred）；timeout/auth/network 不进入冷却 ⇒ 主 provider 持续超时时每轮仍先付其重试预算
（由 PA-171「连续 2 轮失败即停」兜底）；`plan_turn` 同步轮与子代理/静态 resolver 不换手（deferred）。
