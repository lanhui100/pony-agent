# RFC：dev-team 引入「构建并发预算」通用机制（language-neutral）

Status: implemented（v1，配置驱动；SKILL.md 仅两处中立补丁）
Date: 2026-11-17
Scope: `~/.agents/skills/dev-team/`（机制与校验脚本）+ 本仓实例（配置与包装命令）

## Problem

多 Agent 并行的收益被**共享构建设施超订**吃掉：主流构建系统（cargo / npm+turbo / bazel / gradle / go build）的并发锁**按输出目录加锁**，不同输出目录各持一把锁、互不阻塞；每条命令默认又占满整机核数。于是「N 条 lane × 各自输出目录」= N × 核数个编译进程，CPU/链接器/磁盘同时饱和，所有任务接近停摆。

## Evidence（三现律：跨会话/跨仓 ≥3）

| # | 现场 | 观测 |
|---|---|---|
| 1 | 本仓 wave61（`.dev-team/meta-retrospective-wave61-team.md:38`） | 并发编译 `fork: EAGAIN` 反复出现，全量 `cargo test --tests` 无法稳定跑通，被迫改「定向 8 套件」 |
| 2 | 本仓 2026-11-17（B218–B223 波次） | 4 条 target 目录（debug/test/check/release）× 16 job ⇒ load **45.86**（16 核）；release 构建 **90min 未完成**，单跑实测 **3m07s–5m32s** |
| 3 | job_copilot / job_copilot-interview-voice（`.dev-team/report-matrix-motion-wave-a.md:37`） | 并发 workerd/k3s/python 使 load 4.2–6.2，idle 基线自身出现 3–9 个长任务 ⇒ 动效门禁绝对值不可比、判定失真 |
| 4 | 本仓 wave59（`.dev-team/wave59-green-receipts.md:132`） | `T7-PONYGO-OB2-GATE` 在并发负载下 1/4 次假红（audit-diff 判「证据不可得」） |

结论：不是单仓偶发，而是「共享构建设施 + 并行执行者」这一拓扑的结构性成本，满足上升为通用机制的证据门槛。

## Proposal

1. 目标仓声明一份语言中立的配置 `<repo>/.dev-team/build-budget.json`（字段见 `references/build-budget.md` §2）：每 lane 并发上限、并发环境变量、跨输出目录全局锁、重活唯一执行者、nice 档、负载熔断系数、两条包装命令。
2. dev-team 在 **Team-Readiness Gate** 读取该配置：把并发上限与包装命令写进每个 Executor/Test Agent 的初始 Prompt；重活只由 Lead 在 Join Barrier 独跑且与 lane 编译互斥；负载 > 系数×核数即停一条 lane。
3. Phase 2「共享工具链互斥清单」补一条执行面：lane 构建必须走包装命令；禁多 lane 各自 release / 同时开多个输出目录；禁以加大超时或重跑掩盖超订。
4. 机械校验：`scripts/dev-build-budget-lint.py <repo>`（未知键拒绝、类型/范围/脚本存在性校验），Exit 0 才算声明成立。
5. 无配置时按保守默认（每 lane ≤ 核数 ÷ lane 数、跨输出目录加锁、重活独跑）——机制不依赖配置存在。

## Alternatives considered

- **A. 把 `CARGO_BUILD_JOBS`/`flock` 写进 SKILL.md**：落选。语言特异性会污染通用 skill，Node/Go/Bazel 仓直接失效；且违反去特化门禁（`generality-check` 会拒绝）。
- **B. 只写各仓 AGENTS.md，不进 skill**：落选（部分采纳）。仓内规则仍必要，但「派发前必须读预算并注入 Prompt」是编排面的通用动作；只写仓内等于让每个仓各自发明一次。
- **C. 强制所有构建串行（全局单锁）**：落选。牺牲并行收益；预算的目标是「总并发 ≤ 核数×0.75」，不是取消并发。
- **D. 用 `worktree` 强隔离规避**：落选。worktree 只隔离 git index，不隔离构建缓存/CPU；独立 target 目录反而**放大**并发（正是本次事故形态）。
- **E. 以「加大超时/重试」吸收**：落选。超订下重试只会放大负载（与 Anti-Retry Storm 纪律冲突）。

## Acceptance

- `dev-build-budget-lint.py <repo>` Exit 0；坏配置（未知键 / owner≠lead / 相对锁路径 / 包装脚本不存在）必须 Exit 1。
- 包装命令有负样本自检：`--dry-run` 可断言锁路径、并发值、nice 档；缺配置时 fail-closed（exit 2）。
- 派发记录含「每 lane 并发上限 / 重活归属」；`state.json` 或任务板可复查。
- 复盘必须点名「超订倍数」与「单跑 vs 并发」墙钟差。

## Rollout

- 本仓已完成实例化：`.dev-team/build-budget.json` + `scripts/cargo-lane.sh` + `scripts/cargo-lane.spec.sh`（11 条断言全 PASS）+ AGENTS.md 常载规则。
- 其他仓按需新增同名配置即可复用，无需改 skill。
