# Meta Retrospective — Wave w1（Windows 全量首跑扫尾：PTY gating / 状态行相位 / CI 聚合门）

> 机械校验：`dev-meta-lint.py .dev-team/meta-retrospective-w1-pty-status.md`。
> 审查收据：计划阶段 `.dev-team/review-plan-w1-pty-status-{a,b}.json`（dev-review-lint Exit 0）；
> 实施阶段 `.dev-team/review-join-w1-pty-status.json`。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑**：Lead 编排 + 2 常驻 Teammate（Test Agent / Executor）+ 4 个一次性 Subagent（计划审查 2、复检 2）。
  Roster 峰值 3（Lead + 2），符合 ≤8 预算；审查与复检全部走 Subagent，不占常驻槽位。
- **任务板**：6 个任务（T1…T6）均登记 write_scopes 与验收标准；T1/T2 由 Lead 裁决细化出 T5/T6
  （相位根因、覆盖归零），属合理加板而非碎板。
- **信噪比**：审查收据严格 JSON（零套话），复检只回「结论 + 行号 + 命令 + exit code」；
  长文本只出现在 ADR 与元复盘（承载决策，非过程噪声）。
- **Token 效率教训**：本会话跨 9 轮长任务，最大浪费不是单 Agent，而是**多轮 CI 冷启动**
  （~40 min/轮、每轮只剥出一个二进制缺口）。`--no-fail-fast` 是真正的拓扑优化：一次运行换 N 轮等待。
  **改进项：凡 Windows 专有验收，第一轮就用「只跑目标二进制的验证分支」，不要用主 CI 当探索工具。**

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- **真实拦截（保留）**：
  - 治理视角：发现「ADR 0078 注记内嵌正文 = 改写旧决定」——`decisions:check` 绿灯但语义违规
    （R1–R7 只校验 Status/目录/索引）。纯机器门禁会放过。
  - 测试强度视角：发现 **PTY gating 使 4 例覆盖归零**（集成测试只跑 windows job，ubuntu 只跑 `--lib`）——
    静态 diff 完全「看起来合理」。
  - 同一视角发现 **a12 未钉相位**（8 跑 5 红），并纠正「颜色能力档位」误判：根因是
    `status_carousel_started_at = None` 退化为墙钟相位。典型「凭症状命名缺陷」陷阱。
- **误杀/成本**：2 路计划审查各 2 BLOCKING、合计 4 条全部成立（0 误杀）。
- **机器门禁缺口（建议立法）**：① `decisions:check` 增「旧 ADR 正文/Status 零改动、新口径只在
  新篇+blockquote」断言；② `backlog.mjs lint` 要求关闭类任务带可判定 `description`（含验证命令）。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **分权生效**：Test Agent 只写测试（首步加载 `test-expert`）、Executor 只写 CI、Lead 只写
  docs/账本/状态；三方写域无交集（机械审计显示越界文件全部来自其他并发会话，未进本波提交）。
- **一处真实越界（可接受）**：Test Agent 在 T5 顺手改了 `tui_startup_input_guard.rs` 注释（不在 T5 点名范围），
  属同一「假覆盖声明」缺陷族、零断言改动，按同族豁免记录。
- **并行收益与代价**：Test Agent 与 Executor 并行，墙钟≈较长者；但共享工作区把成本推到提交阶段——
  必须逐 hunk 定向暂存并排除其他并发会话的 13 个文件。**改进项：工作区存在并发会话时默认用独立 worktree。**
- **契约账本**：B067（PTY 缺口）+ B064（扫尾总账）在 jsonl 账本维护；ADR 0078→0119 部分取代用
  「Status 下 blockquote + 索引附注」形态，与 0053/0098/0105 先例一致。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **验证分支作为一等工具**：把「只跑目标二进制的 verify 分支」写进 Phase 2 门禁路径，替代「主 CI 当探索器」
   （实测验证分支 8–12 min/轮 vs 主 CI ~40 min/轮）。
2. **「gating 必须配执行点」硬规则**：凡对某平台 `ignore`，必须同提交为其至少一个**其他平台**补执行点，
   否则判 BLOCKING（本轮靠人看才发现）。
3. **症状命名禁令**：缺陷归因必须给「机制 + 反证命令」；「颜色能力档位」这类无机制命名应直接打回。
4. **ADR 部分取代的机械影子**：lint 断言「旧篇正文与 Status 行零改动；新口径只在新篇 + blockquote」。
5. **并发会话写域隔离默认值**：`git status` 出现非本波文件时强制走 `git worktree`（本轮以逐 hunk 暂存兜底，
   依赖人工判断、不可复现）。

## 附：本波验收结果（机器凭证）

- **Windows 全量验收**（run 43003138264，`--no-fail-fast`，PR #17 验证分支）：本波 5 个目标二进制全绿——
  `tui_startup_input_guard` 0 passed/2 ignored（gating 生效）、`tui_overlay_cancel_acceptance` 119 passed/2 ignored、
  `tui_render_layout` 115 passed/0 failed、`tui_dialog_wave39` 118 passed/0 failed、`tui_acceptance` 140 passed/0 failed。
  全量套件仅剩 1 个 scope 外失败（`storage_stage0_write_amplification`，登记 B072）。
- **Linux 本地**：四二进制 2/141/135/138 全绿；`tui_dialog_wave39` 连跑 8 次 0 红（审查基线曾 8 跑 5 红）；
  `npm run decisions:check` 120 篇一致 Exit 0；`node scripts/backlog.mjs lint` PASS。
- **复审**：计划审查 2 路（各 2 BLOCKING，全部成立、0 误杀）→ 修复 → 复检 A 3/4 闭合并指出账本残留（已修）→
  复检 B PASS（2 条 ADVISORY 已修）→ L3-P 合入前评审 1 BLOCKING（账本假承诺）+3 ADVISORY，均已修并落
  `review-join-w1-pty-status.json`（degraded 转录，带 closed_findings）。
- **交付**：`9da23152` + 后续收口提交；账本 B064（扫尾总账）/B067（PTY 缺口）/B072（新发现 Windows-only）。
