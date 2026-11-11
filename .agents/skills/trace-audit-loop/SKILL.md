---
name: trace-audit-loop
description: 何时用：用户要启动/继续 trace 审核 loop、推进审核轮次，或需按 ADR 0101 从断点恢复时使用。触发词：启动/继续 trace 审核、trace 审核 loop、audit loop、推进审核轮次。凭 docs/audit/loop-state.json 断点续跑，推进到下一停点即止；批准是唯一人工节点，verdict 只认脚本退出码。
---

# Trace 审核 Loop 程序

真值源：`docs/audit/loop-state.json`。设计正典：`docs/decisions/0101-trace-audit-loop-automation.md`。人读契约：`docs/audit/README.md`。

## 真相源

- 状态真值：`docs/audit/loop-state.json`（lease / phase / round / budget）。
- 设计正典：`docs/decisions/0101-trace-audit-loop-automation.md`；人读契约：`docs/audit/README.md`。
- 机器证据：`scripts/audit/*` 的实际输出，以及 `docs/audit/rounds/<round>/receipts/` 下的逐阶段回执；叙述与状态文件冲突时以回执和退出码为准。

## 步骤（入口程序）

1. 读 `loop-state.json`。`lease` 未过期且非本会话 → 报告双跑冲突，no-op 退出。
2. 取/续 lease（本会话 id + 30 分钟到期），CAS（expectedHash）写回。
3. 按当前 `phase` 执行下表程序；**每次阶段迁移**都写回执到 `docs/audit/rounds/<round>/receipts/`（序号-阶段-节点：命令、退出码、产物路径、时间戳），再 CAS 迁移状态。
4. 到停点即止：`await_approval` / `escalated` / 预算耗尽。释放 lease，向用户报告当前状态与下一步人工动作。verdict pass 时棘轮后继续下一维度，直至新提案待批。

## 阶段程序

| phase | 程序 | 迁移 |
| :-- | :-- | :-- |
| collect | P0 人工：`bash scripts/observability/check-langfuse.sh --json` + 收集该维度证据会话（≥3，记 session/trace id）；P1+：`scripts/audit/fetch.sh` / `replay.sh`。UNKNOWN ≠ PASS | → audit |
| audit | 根因分析，必须引用证据 span/session id；可派只读子智能体隔离上下文 → `root-cause.md` | → propose |
| propose | 写 `proposals/<round>.md`：frontmatter 含 status: proposed、dimension、阈值、断言命令、修复方案、写域、回滚路径；记录 corpus_hash / assertion_hashes 待批准冻结 | → await_approval |
| await_approval | 读提案 frontmatter：approved → implement；rejected → escalated（或按提案指示跳过）；proposed → **停点 no-op** | 见左 |
| implement | attempt += 1；attempt > 3 → escalated。最小闭包修改 + 定向单测，写域限提案声明；P0 人工驱动 | → build |
| build | P0 人工：`npm run verify` + cargo build + 本机安装；P1+：`build-install.sh` | → verify |
| verify | scratch worktree 双臂重放 corpus（每臂 ≥3 会话）+ fetch 指标；P0 人工等价执行并诚实记 node=manual | → verdict |
| verdict | 跑批准提案中的断言命令，只认退出码；P1+：`verdict.sh`。**禁止口头判 pass** | pass → ratchet；fail → implement（attempt<3）/ escalated |
| ratchet | 断言入棘轮集（单调增长）；取队列下一维度开新 round | → collect；队列耗尽 → mode=patrol |
| patrol | 棘轮全集 + corpus 重放对当前 release；回归 → 受影响维度开新 round（mode=queue） | 见左 |

## 纪律（防作弊）

- 状态迁移一律 CAS；回执只认机器证据（退出码/断言输出），不认叙述。
- 阈值、corpus_hash、assertion_hashes 批准时冻结；事后放宽 = 新提案重审。
- implement 前必须机械验证 frontmatter `approved`；未批准实施 = 违规。
- escalated 状态人工重置前，唤起即 no-op。
- replay 只在一次性 scratch worktree（固定提交）内跑，不污染主仓；corpus 任务只读。
- 凭据只走管道/env，不进 argv；UNKNOWN（退出码 3）永远不等于 PASS。
- P0 阶段脚本节点未建成时用人工等价命令，回执如实标注 `node: manual`，不得伪称脚本已存在。

## 校准样例

- 正例：提案 frontmatter 为 `proposed` 时停在 `await_approval` 并报告人工动作；只有读到 `approved` 才进入 implement。
- 反例：未批准就改代码；或把 UNKNOWN（退出码 3）、口头"看起来通过"当作 verdict pass。
- 正例：每次阶段迁移写回执（命令、退出码、产物路径、时间戳）后再 CAS 迁移；失败如实降级到 escalated。

## 验证与报告

- verdict 只认批准提案中的断言命令退出码；报告写明当前 phase/round、lease 状态、证据 session/trace id、断言命令与 exit、下一步人工动作。
- 未建成脚本的 P0 节点按人工等价命令执行，并标注 `node: manual`；不得伪称脚本已存在。
- 完成后复核 `loop-state.json` 与回执一致；不一致按状态机恢复，不手改叙述。
