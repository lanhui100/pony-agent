# Agent Note: 队友子进程带角色 git 身份（提交归属可对账）

Status: implemented

## Problem

dev-team 编排下，队友 turn 内的 `git commit` 与 Lead 完全同源：Run 走 `SandboxRequest` 的
`environment_allowlist`（`isolate_environment = true` 时先 `env_clear`，只回填 `ESSENTIAL_ENV_VARS`
与 allowlist），JobStart 直接继承父进程环境 —— 两条子进程通路都没有任何身份信息，于是 commit 的
author 恒为宿主 `user.name`。

实证（`~/job_copilot` 2026-11-24 批次，转登 `backlog:B1795535505312` 证据③）：该批 4 个提交 author
全为占位 `t <a@b>`，`git log` 无法区分 Lead 与队友产出，交付归属不可审计。这也是「dev-team 编排
是否真的生效」无法用机器证据回答的直接成因之一：状态面已由 `backlog:B1795535505312` 修好，但
「谁写的」仍然只能靠 Lead 自述。

## Decision

把「队友会话」翻译成 git 认的身份环境变量，在**两条子进程通路**同源注入：

- 新增 `agent/teammate_identity.rs` 作为唯一判据：`teammate_git_identity(target)` 给出
  `teammate:<target>` / `<target>@teammate.pony-agent.local`；`git_identity_env_for_session(session_id)`
  以 `teammate_target_for_session`（`register_teammate` 预分配的固定会话 id 的反查）为唯一真源，
  返回 `GIT_AUTHOR_NAME/EMAIL` + `GIT_COMMITTER_NAME/EMAIL` 四元组；`git_identity_allowlist_for_session`
  产出 Run 沙箱认的 `KEY=VALUE` 形态。模块**不维护任何成员表**。
- Run（`agent/tools/run.rs`）：`SandboxRequest::environment_allowlist` 由常量空表改为上述 allowlist。
- JobStart（`agent/jobs.rs::job_start_scoped`）：spawn 前把四元组显式 `command.env(...)`（该路径本就
  继承父环境，故只做覆盖）。
- 宿主 / Lead 会话与未登记会话一律返回空表 ⇒ 不注入任何变量，用户的提交习惯零改动。
- author 与 committer 必须成对给：只给 author 会产出「author 是队友、committer 是宿主」的半截身份，
  反而更难对账。

验收：`tests/teammate_git_identity_test.rs` 5 例——身份派生纯函数、宿主会话零注入（含端到端反证）、
四元组同源成对、JobStart 子进程读回 `teammate:<target>`、Run 子进程（沙箱 `env_clear` 后仅
allowlist 生效）同样读回 `teammate:<target>`。

## Alternatives considered

- **只注入 `Co-authored-by` 尾注**：不动 author，看似最不侵入；但尾注写在提交信息里、模型可自由
  省略，且 `git log --format='%an'` 仍然分不出归属 —— 判据退化成「模型有没有记得写」，正是本缺陷
  要消灭的那类软约束。选环境变量（git 自身强制生效）。
- **在 SKILL 里要求队友自己 `git config --local user.name`**：零代码改动，但会污染仓库级配置（下一个
  提交的人莫名其妙变成队友），且队友可以忘 —— 拒绝。
- **给每个队友建独立 git worktree + 独立配置**：归属最干净，但 dev-team 的写域模型是「同一工作树
  按路径切分」，另建 worktree 会引入跨树合并成本，与现有并发契约冲突。
- **只改 JobStart 不改 Run（或反之）**：`git commit` 常走 Run，长任务走 JobStart；只改一条等于留
  一半不可审计路径 —— 两条都改，判据同源。
- **注入真实姓名/邮箱（如队友名当 user.name）**：可读性最好，但会在 `git log` 里冒充真人身份，
  且与宿主真实提交无法用前缀区分。选带 `teammate:` 前缀 + 专用域名的显式伪造身份。
- **新增独立工具 `set_commit_identity`**：模型可见工具面是冻结面（46 项 + schema hash golden），
  新增工具要动 8 处清单与快照，收益只是把同一事实换个入口 —— 拒绝。

## Consequences

- `git log --format='%an'` 可直接区分 `teammate:<target>` 与宿主作者，`git log --author=teammate:`
  能筛出全部队友产出；这是「非执行者测试证据」门禁（`backlog:B1795571368443`）可落地的前提之一。
- 注入只发生在「会话 id 能反查到 teammate target」时；未登记会话（含 Lead、含历史遗留会话）零影响。
- 队友若显式 `git -c user.name=...` 覆盖，仍可绕过 —— 本决策只保证「默认可对账」，不保证「不可伪造」；
  伪造属 review 面。
- 沙箱 `environment_allowlist` 从「恒空」变成「按会话可能非空」，这是 Run 语义的可观测变化：宿主会话
  行为逐字不变，队友会话多四个变量（`tests/teammate_git_identity_test.rs` 双向断言钉住）。
