# Agent Note: backlog 体系落地流程资产化为 backlog-onboarding 技能

Status: implemented

## Problem

账本体系（契约 + CLI + 门禁）已在 pony-agent 落地并通过 8 轮独立验收，但「如何把它接进一个仓/worktree」
只存在于本轮对话与提交历史里。接下来要复用到其它工作区（还有 13 个旧体系仓 + 任意新仓），没有固定程序时
每仓都会重新踩同一组坑：

- 同一仓库的多个 worktree 共享账本（`--git-common-dir`），重复 init 会造出第二份真源；
- `core.hooksPath` 不随 `git clone` 传播，新 clone 的本地门禁静默失效；
- 旧体系（`backlog/backlog.md` + `scripts/taskctl.py`）迁移需要冻结、重编号、回填证据；
- CI 里必须有随仓提交的 CLI，否则门禁脚本寻址不到；
- 编号分配下限被硬编码为 56（pony-agent 迁移遗留），别的仓复用会「从 B056 起」。

把程序留在对话里等于没有程序：下一个人要重新推导，且推导结果无法机械校验。

## Decision

1. 技能放**全局技能目录** `~/.agents/skills/backlog-onboarding/SKILL.md`（跨仓可路由），仓内不保留副本；
   按 `.agents/skills/README.md` 的五要素契约组织：触发式 description / 真相源 / 步骤 / 注意事项 /
   校准样例 / 验证与报告。目标仓的契约、CLI、门禁仍是各仓版本控制资产，技能只提供程序。
2. 技能覆盖三类接入判定：**同仓新 worktree**（共享，只做校验）、**新仓 init**（数据骨架 + 接线）、
   **旧体系迁移**（盘点 → 转换 → 冻结 → 验账 → 留痕）。
3. 「注意事项」显式登记五条实测坑：worktree 共享账本、`.locks` 不入仓、编号下限 56、只经 CLI 改账本、
   `hooksPath` 不随 clone、CI 需随仓 CLI、无 node 时 fail-closed。
4. 「校验」给可直接复跑的收据命令（lint / doctor / gate / spec / hooksPath + 临时 clone 端到端），
   并要求贴 exit code；迁移仓额外贴条数与高水位。
5. 编号下限 56 不在此次改动里硬改（会动 CLI 分配语义与既有编号），改登记为账本待办，由下一次决策处理。

## Alternatives considered

- **写进 `backlog/README.md`**：契约仓放的是「模型/判定口径」，放流程会让「步骤改动」与「契约改动」纠缠；
  README 也不进技能路由，agent 不会在「要接新仓」时自动读到 → 落选。
- **放仓内 `.agents/skills/backlog-onboarding/`**：只对 pony-agent 及其 worktree 生效，复用到别的仓要再
  复制一份（复制即漂移），且会被 ponygo 计入「实例自有」资产（形状判据 3.4 还需额外维护）→ 落选。
- **做成 ponygo 模板库技能（`.meta/skills-library/pg-*`）**：这是跨仓通用程序、未来可能值得进库，但当前
  仍含 pony-agent 特化事实（仓库路径、gate 脚本名），先做实例自有技能；进库需先剥离特化 → 落选。
- **写成一键 shell 脚本**：安装是破坏性动作（hook 接线、gitignore、CI 编辑），跨仓差异大；本轮用户明确
  「先不考虑自动化落」，先固化人工程序与校验，自动化另立决策 → 落选。
- **只留在 ADR/对话**：ADR 是决策不是程序，且不会被路由到「正在接新仓」的时刻 → 落选。

## Consequences

- 全局新增技能一篇（不进本仓版本控制，`ponygo audit` 的实例自有仍为 4 篇）；新增账本待办一条
  （编号下限参数化）。
- 复用方式：接新仓时加载该技能，按「第 0 步判定 → 对应分支 → 校验」执行；不满足校验即停。
- 已知风险：全局目录不在任何 git 仓内，技能演进不可审计——与 2026-11-12 讨论的「全局资产应有独立
  git 仓」同源，后续应把 `~/.agents/skills/{backlog-onboarding,pg-backlog}` 收敛到该仓。
- 诚实边界：技能里的命令是已在 pony-agent 实测过的路径；其它仓的 CI 平台（非 GitHub）与 Windows
  hook 行为未在技能中验证，需在目标仓补一次端到端；自动化安装仍未实现。
