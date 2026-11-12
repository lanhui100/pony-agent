# Agent Note: backlog 体系技能收敛为单一 pg-backlog（onboarding/legacy 收进 references）

Status: implemented

## Problem

本轮先新增了独立技能 `backlog-onboarding`，随后复盘发现三处结构性问题：

1. **一个体系两篇技能**：`pg-backlog` 与 `backlog-onboarding` 共享同一真相源（目标仓契约 + `scripts/backlog.mjs`），
   拆开后两篇都要维护同一组命令与口径，天然制造漂移——正是本项目在 ADR 里反复修过的病。
2. **旧技能仍在教旧模型**：`pg-backlog` 正文仍以 `backlog/backlog.md`、`scripts/taskctl.py`、`.lane` 为主，
   与 JSONL 新模型冲突；agent 若先路由到它就会写错载体。
3. **路由冲突**：两篇都含「backlog」触发词，同一次请求可能只命中旧模型那篇。

## Decision

1. **只保留一篇全局技能** `~/.agents/skills/pg-backlog`：SKILL.md 负责触发、真相源、**模型版本判定**
   （`tasks.jsonl` vs 旧 md/taskctl）、日常操作、门禁与校验；低频细节放 `references/` 懒加载。
2. `references/onboarding.md`：新仓接入（数据骨架、CLI/gate 随仓、hooksPath、CI）与旧账本迁移步骤。
3. `references/legacy-model.md`：旧 md + taskctl 模型说明与迁移规则，**只服务迁移期**；新仓禁止再采用。
4. 删除独立技能 `backlog-onboarding`；仓内不保留任何技能副本（全局目录是唯一家）。
5. `pg-backlog` 的旧正文（OpenSpec/taskctl/lane 为主）退役，仅以 legacy 参考形态保留。

## Alternatives considered

- **保留两个技能**：同体系双真源、触发词重叠，维护面翻倍 → 落选。
- **只扩 `pg-backlog` 单文件、不放 references**：接入/迁移细节占一半篇幅，日常操作被稀释；
  技能的五要素要求「真相源清单 + 步骤」可执行，长尾细节应懒加载 → 落选。
- **把技能放回仓内 `.agents/skills/`**：只对 pony-agent 及其 worktree 生效，复用到别的仓要再复制 →
  与「技能全局、契约/CLI/门禁随仓」的分层冲突 → 落选。
- **先不改 `pg-backlog`、等独立 git 仓建好再改**：旧模型正文会继续误导其它仓 agent；先收敛正文，
  版本化问题另案处理 → 落选。

## Consequences

- 全局技能从 2 篇（含旧正文）收敛为 1 篇 + 2 个 references；dsh 技能目录已刷新，`backlog-onboarding` 不再存在。
- 旧模型仓的行为不变：技能第 0 步会要求先迁移，迁移前不按新模型操作。
- 已知风险（与前一条 ADR 同源）：全局目录不在任何 git 仓内，正文与 references 的演进不可审计；
  后续应把 `~/.agents/skills/pg-backlog` 收敛到独立 git 仓，并以脚本做模板同步。
- 诚实边界：references 里的命令基于 pony-agent 实测；其它仓的 CI 平台与 Windows hook 行为仍需在目标仓复核一次。
