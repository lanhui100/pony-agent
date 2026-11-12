# Agent Note: backlog 账本契约回家与门禁登记（backlog-gate + 负样本 spec）

Status: implemented

## Problem

账本本体（单一 JSONL + CLI + 租约）已于 2026-11-11 统一（见
`2026-11-11-unified-backlog-jsonl-ledger.md`），但配套的「契约 — 门禁 — 治理视图」三件事没有收口，
实测出四处缺口：

1. **契约无家**：字段、状态机、租约、归档口径只存在于 ADR、`scripts/backlog.mjs` 代码与仓外全局 skill
   （`~/.agents/skills/pg-backlog/SKILL.md`；其核心操作节仍在教已不存在的 `backlog/backlog.md`
   与 `scripts/taskctl.py`）。新加入者/agent 每读一次要拼三处，且 ADR 是历史时点记录、不能随手更新。
2. **门禁未登记**：`backlog lint` 虽双挂载（`.githooks/pre-commit` + `ponygo-governance.yml`），但没有任何
   门禁索引登记它（`.meta/gates/README.md` 是骨架文档、逐字节锁定），也没有同 Stem 的负样本 spec——
   治理视图里看不见这道门，判据 2.3 的配对形态也不成立。
3. **对账门虚设**：`doctor`（跨归档 ID 去重、高水位、孤儿租约）没接任何门禁。实测现场残留
   `backlog/.locks/--help.lock` 使 `doctor` 长期 exit 1 而无人发现（本次 `reap --minutes 0` 已回收）。
4. **一键自检漏检**：`npm run verify`（AGENTS.md 宣称的「全量合规自检」）不含账本校验。

## Decision

1. **契约回家**：`backlog/README.md` 是本仓 backlog 的唯一可读契约（布局 / 必填字段 / 状态机 / 租约 /
   归档 / 命令 / 门禁），并显式声明「判定真源 = `scripts/backlog.mjs`，本文不复刻判据」。
   全局 skill 只保留跨仓通用模型，冲突以仓内契约为准。
2. **门禁统一实现**：新增 `.meta/gates/backlog-gate.sh`，语义 = `lint`（记录不变量）+ `doctor`
   （跨归档 / 高水位 / 孤儿租约对账）。账本不存在时明确跳过（迁移期友好）；无 `node` 时 fail-closed。
3. **负样本配对**：新增 `.meta/gates/backlog-gate.spec.sh`，在 `mktemp -d` 夹具仓中断言
   「非法账本被拒 / 合法账本放行 / 无账本跳过 / 无 node 拒绝」，满足判据 2.3 的同 Stem 配对。
4. **三处接线**：pre-commit（哨兵块之外，判据 2.4 要求块内逐字节等于规范块）、CI
   （`ponygo-governance.yml` 独立 step；`ci-verify.sh` 属框架不可变资产，不动）、
   `package.json`（`backlog:gate` 并入 `verify`）。
5. **治理视图登记**：新增 `.meta/gates/INSTANCE-GATES.md` 登记该实例门、spec 配对与「承诺 → 命令」；
   不改 `.meta/gates/README.md`——它在 `UPGRADE_UNIVERSE` 内（逐字节锁定），追加实例内容会让
   `ponygo upgrade --check` 长期报漂移（本次实测 rc=1），登记收益不足以换永久骨架漂移。

## Alternatives considered

- **契约留在 ADR**：ADR 记「当时为什么」，随状态迁移冻结、不允许随手更新；每读一次要翻 200 行历史 →
  落选。
- **契约写进全局 skill 正文**：skill 是程序资产（Rule of three），pony-agent 特化事实（OpenSpec 退役、
  lint 双挂载）不该污染跨仓共享文件；全局文件不在本仓版本控制内，改坏无审计 → 落选。
- **只登记 lint、不接 doctor**：lint 看不见孤儿租约与跨归档对账，现场那条 `--help.lock` 正是只有
  doctor 才抓得到的脏态 → 落选。
- **gate 内复刻 lint 判据**：两份实现必然漂移；`scripts/backlog.mjs` 是唯一判定真源，gate 只做编排 →
  落选。
- **把登记写进 `.meta/gates/README.md`**：它在骨架宇宙内、逐字节比对，追加一行即让
  `ponygo upgrade --check` 常红（实测 rc=1）；登记是文档面需求，不该用永久漂移换 → 落选。
- **把 gate 塞进 `ci-verify.sh`**：A4 属框架不可变资产（`ponygo upgrade --check` 逐字节比对），
  改它即制造永久 drift → 落选。

## Consequences

- 新增四文件：`backlog/README.md`、`.meta/gates/backlog-gate.sh`、
  `.meta/gates/backlog-gate.spec.sh`、`.meta/gates/INSTANCE-GATES.md`；四处接线：pre-commit、
  CI workflow、`package.json`、实例门登记表。
- 门禁语义：账本缺失 → 跳过（迁移期）；账本存在但非法 / 有孤儿租约 / 高水位越界 → 拒绝提交。
- 全局 skill（`~/.agents/skills/pg-backlog`）仍含旧模型段落；跨仓替换涉及其他 13 个工作区，
  属独立决策，本 ADR 不覆盖。
- 诚实边界：spec 只证明「门会拒非法样本」；账本内容真实性靠 review。`doctor` 不解析 git 历史，
  无法识别「同时改记录与高水位」的伪装。
