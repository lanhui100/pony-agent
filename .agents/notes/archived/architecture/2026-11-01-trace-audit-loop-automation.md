# Agent Note: Trace 审核 Loop：用户唤起 + 持久状态机 + 确定性骨架 + 单一审批门

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0101-trace-audit-loop-automation.md` 之归档快照，仅供溯源。

## 背景

七维度 trace 审核矩阵（时延结构 / 上下文经济 / 失败语义 / 行为质量 / 成本治理 / 上报链路自检 / 跨宿主一致性）与「分批审核 → 提案 → 批准 → 实施 → 构建安装 → 重跑验证」的 loop 已获批准。本 loop 是**中期持续机制**而非一次性任务：由用户随时唤起、凭持久状态断点续跑，维度队列走完后转入回归巡检。

可复用的既有资产（物理事实）：

- **headless 执行器已在主干**（PA-164，`crates/pony-agent-core/src/bin/cli.rs`）：`--output-format jsonl|sse`、退出码分类学（0/2/3/4/5/6/7/1）、`--result-json` 终态信封、`--permission-mode`（默认 deny）、`--timeout-secs` 有界执行。
- **trace 查询通道已有**：`scripts/observability/check-langfuse.sh`（Tempo TraceQL + Langfuse API 双轨，退出码 0/2/3，纪律 UNKNOWN ≠ PASS）。
- **闭环先例**：`docs/analysis/trace-analysis-*` → ADR 0055；棘轮先例 ADR 0090；通知通道 `scripts/send-wecom-notification.mjs`。

## 候选方案

### 触发与编排形态

- **A 用户唤起 + 持久状态机 + skill 程序（选中）**：用户在任意会话说「启动/继续 trace 审核」即加载 `.agents/skills/trace-audit-loop/SKILL.md`，读 `docs/audit/loop-state.json` 推进阶段，到停点落回执退出；也可用 `ponycli` 无头命令单步唤起。中期性由状态持久化 + 巡检模式保证，不依赖调度器。批准反馈明确：节奏由人掌控，不必是定时任务。
- **B 定时器 + headless 无人值守**：初版提案曾选中，批准时降级为**可选演进**（不列验收）。落选理由：审核节奏应由人唤起；无人值守放大假 PASS 与未批准实施的风险面；状态机已保证随时唤起可续跑，cron 不是必需品。
- **C 会话内 `create_goal` 自动续行**：goal 的 pause/resume 要求人类意图，与「等待审批」语义错配；外部长会话上下文膨胀，跨会话续跑最终仍靠状态文件。落选。
- **D 纯 TS/Rust 确定性驱动 + 直调 LLM API 做判断节点**：确定性最高，但需重造工具循环、治理、trace 上报整套 harness，违反复用原则。落选：保留为稳定后的演进选项。
- **E CI（GitHub Actions）编排**：构建安装目标是本地开发机，Tempo/Langfuse 为本地网服务，审批交互不适合 CI 形态。落选。

### 审批门形态

- **文件 frontmatter（选中）**：提案文件 `status: proposed → approved/rejected`，机械可查、git 可审计、跨会话稳定。
- **聊天内回复批准**：不可机械检查、跨会话即丢失。落选。
- **PR review 门**：重量级且与本机安装验证脱节。落选（后续可叠加，不互替）。

## 决策

Loop 采用「**用户唤起 + 持久化状态机 + 确定性脚本骨架 + Agent 判断节点 + 单一审批门**」：机械可查的一切做成非零退出脚本，需要判断的做成 Agent 步骤，批准是唯一人工节点；verdict 只由脚本退出码计算，Agent 无权口头判 pass。

### 唤起与停点

- 入口：`.agents/skills/trace-audit-loop/SKILL.md`（Agent 程序）；`docs/audit/README.md` 是人读契约。
- 每次唤起推进到**下一个停点**即止：`await_approval`（提案待批）/ `escalated`（超限或否决）/ 预算耗尽；verdict pass 则棘轮后继续下一维度直至新提案待批。
- 跨会话续跑只依赖状态文件，不依赖对话记忆；loop 不要求常驻 dev team（team 状态会话内即失忆，多 writer 破坏写域冻结）。audit 节点可按需派只读子智能体做根因分析。

### 状态机（单一真值源）

`docs/audit/loop-state.json`，阶段流转：

```
collect → audit → propose → await_approval → implement → build → verify → verdict
  ├─ pass → ratchet → 下一维度 collect（队列耗尽 → patrol）
  ├─ fail 且 attempt < 3 → implement（携失败证据重试）
  └─ fail 且 attempt ≥ 3 / rejected → escalated（人工重置前唤起即 no-op）
```

字段：`mode / dimension / round / phase / baseline_release / candidate_release / corpus_hash / assertion_hashes / evidence_sessions / proposal_path / approval / attempt / receipts[] / lease`。每次迁移以 CAS（expectedHash）写入；`lease`（session id + 到期时间）防双唤起并发推进，阶段迁移续租，过期租约可被新唤起接管。

### 中期持续：队列 + 巡检双模式

- **queue**：按序 6 → 1/3 → 2 → 5 → 7 → 4 逐维度推进，每维度独立 round，通过后断言入棘轮。
- **patrol**：队列全部通过后，每次唤起对当前 release 跑棘轮全集 + corpus 重放；回归即对受影响维度开新 round。新维度经批准提案入队。loop 因此是中期机制，不是一次性工程。

### 确定性节点（每条承诺配非零退出命令）

| 节点 | 落点 | 职责 | 退出语义 |
| :-- | :-- | :-- | :-- |
| corpus | `docs/audit/corpus/` | 固定可重放任务集，批准时 hash 冻结 | hash 漂移 = 红 |
| replay | `scripts/audit/replay.sh` | `ponycli --output-format jsonl --result-json … --permission-mode … --timeout-secs`；每臂 ≥3 会话，baseline/candidate 双臂同任务集 | 进程失败/退出码异常 = 红 |
| fetch | `scripts/audit/fetch.sh` | TraceQL/Langfuse API 按审计标签拉会话 → `metrics.json` | 缺凭据/不可达 = 3（UNKNOWN），绝不 PASS |
| assert | `scripts/audit/assertions/dim<N>.sh` | 维度阈值断言；阈值随提案批准立法并冻结 | 超阈 = 非零 |
| ratchet | `scripts/audit/ratchet.sh` | 已通过维度断言全集，单调增长（ADR 0090 同款纪律） | 任一红 = 阻断 |
| build-install | `scripts/audit/build-install.sh` | `npm run verify` + cargo build + 本机安装替换 | 非零 = 红 |
| verdict | `scripts/audit/verdict.sh` | assert + ratchet + 前后对照 → pass/fail 回执 | 唯一裁决者 |

审计臂归因：复用既有 session/release 归因键注入 `audit.round / dimension / arm` 标记；若现有 span 属性面不足以承载，接线点经 ADR 0068 契约版本化通道扩展，P1 实施时确认，不另造私有 schema。

### 重放隔离（scratch worktree）

replay 双臂会话在一次性 git worktree（`git worktree add` 至 scratch 路径）中执行：baseline 臂固定 round 开始提交，candidate 臂固定实施提交；初始状态确定、不污染主仓，round 收口后移除。corpus 任务限只读或仅写该 worktree 内路径；构建安装目标仍是本机（dev 服务器即本机）。

### 判断节点（Agent 步骤）

skill 定义程序与结构化产出：

1. **audit**：根因分析，必须引用证据 span/session id → `root-cause.md`；
2. **propose**：提案 frontmatter 含维度、阈值、断言命令、修复方案、写域、回滚路径 → `proposals/<round>.md`；
3. **implement**：最小闭包修改 + 定向单测，写域限提案声明范围。

产物统一落 `docs/audit/rounds/<id>/`。

### 审批门（唯一人工节点）

人工将提案 frontmatter 置 `approved`（或 CLI 辅助）后 loop 才继续；未批准时唤起停在 `await_approval` no-op，`rejected` 则跳过或升级。审批请求与 escalated 通知复用 `send-wecom-notification.mjs`。

### 防作弊与安全

- 阈值、corpus hash、断言 hash 在批准时冻结；事后放宽阈值 = 新提案重审。
- 每维度修复尝试 ≤3 次，超限 escalated，人工重置前不再自动实施。
- 轮内冻结检查：提案写域之外的 harness diff → 门禁红。
- 预算：每次唤起 `--timeout-secs` 上界 + 既有治理成本窗口（ADR 0048）。
- UNKNOWN ≠ PASS，继承 `check-langfuse.sh` 纪律。

### 分阶段落地（进度归任务系统，不影响本 ADR 状态）

- **P0（PA-168）**：loop-state + corpus + skill 入口 + 维度 6 断言，人工驱动各阶段（测量地基先行，维度 6 未过不得自动化其上）。
- **P1（PA-169）**：replay/fetch/assert/build/verdict/ratchet 节点脚本化。验收：维度 6 首个完整 round 机械判 pass。
- **P2（PA-170）**：一次唤起完整推进一轮至下一停点（含 headless 单命令形态）+ patrol 巡检模式。验收：连续 3 次唤起均机械推进至下一停点，零未批准实施、零假 PASS。定时器为可选演进，不列验收。

## 影响

- loop 会话复用 ponycli 既有 trace 出口，自身成为维度 6 的审计对象（自举）；归属说明已落 `docs/observability.yaml` note。
- 任务卡 PA-168/169/170 登记 `management/task-system/`，串行依赖。
- ratchet 套件将成为常驻门禁，接线 `npm run verify` 或独立 audit gate。
