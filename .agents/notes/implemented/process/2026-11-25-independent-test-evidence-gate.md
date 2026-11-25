# Agent Note: 非执行者测试证据的机械门禁（Test-Evidence trailer）

Status: implemented

## Problem

dev-team SKILL 的强制派生条款要求 B 级任务派生 Protocol Agent / Test Agent + Executor，且
`test-agent` 首步加载 `test-expert` 跑 L2-T / L2-AT 双模态红相。但这条只是**流程约束**：
模型可以只派生 executor，再用「执行者自跑 pytest/ruff exit 0」放行——单点自证。

实证（`~/job_copilot` 2026-11-24 批次，转登 `backlog:B1795535505312` 证据①）：该批只派生
2 个 executor，无 test-agent、无 protocol agent，放行依据是执行者自己跑的 6 组命令全 exit 0。
仓内没有任何东西能判红：`backlog:B1795571368443` 记的就是这个缺口。

## Decision

新增一个只判「证据来源是否独立于代码作者」的机械门：

- **载体**：提交信息 trailer `Test-Evidence: <producer> | <command> | exit=<code>`。
  用提交信息而不是另造回执 JSON：它随代码入库、review 后被改必留痕、`git log` 直接可读，
  不制造第二份真值（本项目「每个事实一个家」）。
- **门禁**：`scripts/check-independent-test-evidence.mjs`，判据三条——
  ① 范围内存在 dev-team 标记（trailer `Team-Run:` 或提交信息含 `dev-team`）才进入判定，否则跳过；
  ② 范围内若改动了代码面（非纯文档/账本路径，口径与 `scripts/ci-scope.mjs` 的 SAFE 树同源），
  必须至少有一条 `Test-Evidence`；
  ③ 该证据的 `producer` 必须**不在**本范围的作者集合内（严格字面比较，不做模糊匹配）。
  退出码：0 通过/跳过；1 违例；2 无法判定（base 解析失败）。
- **自检**：`scripts/check-independent-test-evidence.spec.sh`，P1–P7 正负样本 + N1–N2 边界
  （非执行者放行 / 执行者自证判红 / 缺证据判红 / 无团队标记跳过 / 纯文档跳过 /
  字面标记同样触发 / 混合证据有一条独立即放行 / 无法解析 base fail-closed / `--help` 可用）。
- **接线**：CI `ponygo-governance` 作业跑 spec + 本体（base 取 PR base sha，push 取
  `github.event.before`）；`package.json` 增 `contract:test-evidence` /
  `contract:test-evidence:selftest` 两个入口供本地复现。

不做的部分：本门**不重跑**命令（那是 CI 的活），也不保证 producer 真由该角色跑过（伪造属 review 面）。
它保证的是「要么有一条显式的非作者证据，要么红」——把「忘了找独立验证」从静默变成必须显式面对。

## Alternatives considered

- **另造 `.dev-team/test-evidence/*.json` 回执文件**：结构化更好判，但新增一份与提交信息并行的
  真值，且文件可被后续提交悄悄替换而提交信息不动 —— 正是本项目反复清理的「双真值」形态。选 trailer。
- **强制 CI 自己跑测试并核对输出哈希**：判别力最强，但要求门禁知道「哪条命令该跑」，等于把
  dev-team 的波次契约塞进 CI，跨仓无法通用（本门要能装进任意仓）。拒绝。
- **只靠 SKILL 措辞加一句「必须找 test-agent」**：零代码，但已有措辞本来就这么写，实测被绕过 ——
  文字约束在「执行者自己也能跑绿」时没有任何判别力。
- **按提交 author 邮箱域判定（配合 `teammate:` 身份）**：与
  `backlog:B1795571360795` 的 git 身份注入天然衔接，但要求所有团队都在本仓跑（外部 CI / 人工
  提交拿不到该身份）。本门取 `producer` 自述 + 作者集合比对，两个事实都在提交信息里，跨环境可判。
- **默认对所有 PR 生效**：判别力更广，但普通修复类 PR 没有独立测试者，会逼出大量
  「producer: ci」的形式化填写 —— 门禁一旦形式化就失去意义。故以 dev-team 标记为触发面。

## Consequences

- dev-team 批次改代码面时，PR 必须带至少一条 `Test-Evidence:`，且产出者不同于作者；
  否则 `ponygo-governance` 红。
- 普通 PR（无团队标记）与纯文档/账本 PR 不受影响（spec 的 P4/P5 钉住这两个方向）。
- `producer` 是自述，本门不抗伪造；与 `backlog:B1795571360795` 的角色 git 身份叠加后，
  「谁写的」与「谁验的」都可从 `git log` 读出，review 有了可对照的两个事实面。
- 浅克隆/新分支首提交（base 不可解析）时本门 exit 2 而非静默通过 —— fail-closed，不假装验证过。
