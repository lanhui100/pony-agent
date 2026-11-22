# Agent Note: backlog 账本 v3——批次日志（write-per-batch, fold-on-read）+ 按批归档封存

Status: proposed

## Problem

v2 账本把全部任务写进同一个 `backlog/tasks.jsonl`，用 `backlog/.next-id` 做全局计数器。并发会话
（不同 worktree / 不同 PR）只要都记账，就必然在同一文件的行尾与同一计数器上撞车：

- 实证 1（2026-11-22）：PR #40（TUI 任务抽屉修复）与 PR #41（runtime-store 拆解）各自记账，
  合并时 `backlog/{tasks.jsonl,.next-id,archive/tasks-2026.jsonl}` 三文件同时冲突，必须人工并集；
- 实证 2：一次 `resolve` 会重写整个 `tasks.jsonl` 并追加归档，任何两个分支同时 resolve 就冲突；
- 结构性原因：账本的**写入单位是「一个任务的一次状态变更」**，而会话的**工作单位是「发布面批次」**
  （一次会话按 release surface 选一批任务一起做、一起提交）。单位错配 ⇒ 冲突是必然，不是偶然；
  逐分支手工解冲突只是把成本转嫁给每个 PR。

## Proposal

1. **写入模型改为批次日志**：`backlog/log/<batchId>.jsonl`，每个会话/发布面批次一个文件，只追加；
   `batchId = <YYYYMMDD>T<HHMMSS>Z-<surface>-<rand4>`，首行 `batch.start`（含 `surface` / `release?`）。
   事件：`record` / `dedup` / `start` / `release` / `annotate` / `resolve`（每行一个 JSON）。
2. **读模型改为 fold**：`scripts/backlog.mjs fold` 把「v2 遗留归档 + 全部活跃批次 + 已归档批次」
   按 `batch.start.created_at`（同刻按 batchId）折叠出现行视图，不落盘；`list --surface <s>`
   直接列某发布面批次内的任务。`backlog/tasks.jsonl` 与 `.next-id` 删除。
3. **账本根 = 当前 worktree 顶层**（不再用 git common dir）：每个分支写自己的批次日志并随 PR 提交，
   归档从各自检出树只读加载。v2 的「跨 worktree 共享单文件」正是并发写冲突的源头，一并退役。
4. **ID 改为时间基**：新任务 `B` + epoch 毫秒（13 位，仍满足 `B\d{3,}`），跨会话按时间取号、
   天然不撞号；旧短 ID（`B265`…）原样保留，`backlog:B265` 引用不变。`done` 仍是终态。
5. **按批归档（照 ADR 归档机制）**：批次内任务全部 `done` 且随版本发布后，`backlog archive`
   把批次文件搬进 `backlog/archive/batches/<year>/` 并追加 `manifest.sha256`（`sha256  <path>`，只追加）；
   `verify-archive` 逐文件校验。归档**只读**，回归只能新批次 + 引用旧 ID。
   触发：发版后 housekeeping 顺手做，或每 50 个已闭环批次/每周兜底；**不归档未闭环批次**。
6. **不引入常驻「总账」文件**：合并成单一总账会把刚消灭的共享写点请回来；人类视图由 `fold`/`list`
   现算。规模到万级再评估「按年只读归档 + 分页」，当前 200+ 条/年量级无需。
7. **迁移与兼容**：`scripts/backlog-migrate-v2-v3.mjs` 把 v2 活跃任务写成一个基线批次 → 临写 MODEL
   → v3 fold 对账（活跃数/done 数一致才继续，否则回滚）→ 删 `tasks.jsonl`/`.next-id`；v2 遗留归档
   保留只读并入 fold。过渡期 `backlog.mjs` 对 v2 账本提供**只读**视图与 lint/doctor（写命令拒绝并
   指向迁移脚本），`backlog-gate` 对 v2 只读兼容放行 + NOTE，不逼死未迁移的仓。
8. **门禁**：`backlog-gate` v3 = `lint`（批次头/事件结构/时间单调/fold 不变量/租约）+ `doctor`
   （ID 重复/孤儿租约/归档清单）+ `verify-archive`（sha256）；负样本 spec 扩为 P1–P3 + N1–N8 + H。
9. **全局技能**：`pg-backlog` 升 v3 模型，同批交付 `batch-log-model.md`、`migration-v2-to-v3.md`
   与参考实现/夹具（本仓作为首个 dogfood 实例，实现提炼后再更新技能，避免技能先于实证）。

## Alternatives considered

- **一任务一文件（`backlog/tasks/B###.json`）**：能消掉行尾汇聚点，但 ID 仍需全局计数器（或时间基），
  且「一个 PR 一批任务」在 diff 里碎成 N 个文件、review/归档没有批次边界；与「按发布面选一批任务」
  的工作单位不匹配。被否。
- **保留单文件 + GitHub Action 自动并集解冲突**：能自动止血，但冲突仍发生、需推回 PR 分支、
  作者还要 pull，且把「账本一致性」外包给 CI 权限；不是根治。被否（保留为极端兜底）。
- **账本移出 PR 流（独立 orphan 分支/本地账本 + CI 校验）**：冲突消失，但破坏「PR 内可审账本」
  与现有 `backlog-gate` 模型，账本与代码变更的因果链断开。被否。
- **把 fold 结果固化成常驻总账文件**：重新引入单一写点；若只由 CI 在 main 生成，又需要「生成物 +
  防手改校验」，且每次归档是全文件大 diff。被否（留作规模到万级的后手）。
- **按时间强制归档未闭环批次**：会产生「刚归档就要补证据/重开」的二次改动，违反归档只读语义。被否。

## Consequences

- 正面：并发会话/PR 的账本冲突从「每次必撞」变为「结构上不存在」；一个批次 = 一个发布面的工作单元，
  PR diff 天然可审；归档有 sha256 封存，篡改即红；v2 仓可只读兼容、渐进迁移。
- 负面/已知限制：工具链重写（`scripts/backlog.mjs`）+ 一次性迁移；ID 形态变化（新 ID 13 位）；
  旧分支仍用旧工具（各自树内自洽，合并后自然升级）；`fold` 需要读全部批次，规模靠按年归档控制。
- 迁移顺序约束：本仓迁移须在其它在途账本 PR（如 #41）合并后进行（删除 `tasks.jsonl` 与改动
  `tasks.jsonl` 的 PR 必然冲突一次），并在同一提交内交付 MODEL + 基线批次 + 删除。
- 观测性：结构性 N-A（纯本地工具链与文档，无网络/后台常驻任务/新指标）。

## Related

- 可读契约：`backlog/README.md`
- 判定真源：`scripts/backlog.mjs`、`scripts/backlog-migrate-v2-v3.mjs`
- 门禁：`.meta/gates/backlog-gate.sh` + `.meta/gates/backlog-gate.spec.sh`
- 全局技能：`~/.agents/skills/pg-backlog`（v3 模型与迁移指引，dogfood 后提炼）
