# Agent Note: backlog 编号分配下限由 56 改为 1（迁移起点改由命令显式声明）

Status: implemented

## Problem

`scripts/backlog.mjs::allocateId` 把候选编号钳在 56：`Math.max(readHighWater, maxSeen+1, 56)`。这个 56 是
pony-agent 2026-11-11 从 legacy triage 迁移、避开旧 `B001–B055` 编号的一次性事实，却被固化进通用工具。
后果：任何新仓接入后第一条任务都从 `B056` 起，与「新账本」直觉冲突；`backlog-onboarding` 技能只能把它当
注意事项写出来（「复用到其它仓前先确认」），而接下来要迁移 13 个工作区，这个解释成本会被重复 13 次。

## Decision

1. `allocateId` 的下限改为 **1**：`.next-id` 存在就以它为准，否则从 `B001` 起。编号单调性与「归档不复用」
   仍由 `.next-id` 高水位保证，不依赖任何常量。
2. 迁移旧账本的起始号改由**命令显式声明**：`import-triage --start <N>`（默认 56 仅为 pony-agent legacy
   兼容保留，并在函数注释中写明用途）。起始号是迁移决策，应出现在命令与 ADR 里，而不是藏在通用代码里。
3. 迁移入口 `import-triage` 参数先校验再动手：`--start` 不是 ≥1 的整数、或 `--from` 指向的旧账本不存在
   → `exit 2` 且不写账本；`--help`/`-h` 只打印用法、**不静默执行迁移**；usage 行补齐 `--from/--start`。
5. 同步更新全局技能 `pg-backlog/references/onboarding.md` 的「编号下限」注意事项：新仓从 `B001` 起；
   迁移仓用 `--start` 显式指定。
4. pony-agent 现账本 `.next-id = 112`，高于任何历史编号，本次改动对它零影响（无重编号、无迁移）。

## Alternatives considered

- **保留 56**：零改动，但每个新仓都要解释「为什么从 B056 起」，且通用工具继续携带某个实例的历史事实 → 落选。
- **加环境变量/配置项控制下限**：为一个一次性事实引入配置面，配置越多漂移越多 → 落选。
- **按 `legacy/` 目录是否存在推导下限**：隐式魔法——同一份代码在不同目录状态下分配不同编号，难解释难测试 → 落选。
- **直接改 pony-agent 历史编号为新规则**：历史编号已被提交信息与 ADR 引用，重编号破坏可追溯性 → 落选。

## Consequences

- 改 1 行分配逻辑 + 2 条单测期望（`B056/B057` → `B001/B002`、高水位 `57` → `2`），并补 `import-triage`
  参数校验用例（非法/缺值 `--start`、缺失 `--from`、`--help` 不迁移、`--start 1` 迁移出 `B001`）；
  `vitest` 36 例全绿。非法 `--start` 断言必须带**存在的** `--from`，否则 exit 2 会来自 from 守卫、
  令 `--start` 校验的变异测试假绿（本 ADR 记录该测试陷阱）。
- 新仓接入从 `B001` 起；pony-agent 后续分配仍从 `B112` 继续，无重编号。
- `normalizeLegacy`/`import-triage` 的默认起始号 56 保留，但定位为「pony-agent legacy 兼容默认」，
  文档与注释写明可显式覆盖；新仓若要走该入口，应传 `--start 1`。
- 技能 `references/onboarding.md` 的对应注意事项已改为「B001 起 + 迁移显式 --start」。
- 诚实边界：`normalizeLegacy` 默认值仍是 56，属本决策未覆盖的遗留便利；若出现第二个使用该入口的仓，
  应把默认值也改为 1。
