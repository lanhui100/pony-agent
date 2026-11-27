# Agent Note: 全零 base 是「无前序提交」哨兵，不是「解析失败」

Status: implemented

## Problem

`ponygo-governance` 的「非执行者测试证据门」step 这样取 base：

```yaml
node scripts/check-independent-test-evidence.mjs \
  --base "${{ github.event.pull_request.base.sha || github.event.before }}" \
  --head "${{ github.sha }}"
```

在 `push` 事件上 `github.event.pull_request` 为空，于是走 `event.before`。而**新分支的首次 push**，
GitHub 给 `event.before` 的值是 40 个 `0` 的哨兵 sha（语义：这条 ref 上没有任何更早的提交）。

`check-independent-test-evidence.mjs` 的 `resolveCommit()` 把「空 / 全零 / 不可解析」一律折叠成
`null`，`main()` 再据此 `return 2`（fail-closed）。于是**每个新分支的首次推送都稳定拿到一条
`failure` 的 governance check**。

实测（2026-11-27，PR #109 / 分支 `fix/goal-stop-copy-and-status-bar-icon`，提交 `ba765187`）：

| run | 事件 | base | 结果 |
|---|---|---|---|
| `45979310259` | `push`（新分支首推） | `0000000…` | **fail**：证据门 step `exit 2` |
| `45979372445` | `pull_request`（同一提交） | 有效 `base.sha` | pass |

`PR #107` / `#108` 未复现，是因为它们 push 时分支已存在（`before` 是有效 sha）。

危害不在「漏判」而在「假红」：同一提交的 `pull_request` 事件用有效 `base.sha` 照常判定，
判定责任没有缺口；失败的只是那份**结构上不可能成立**的 `push` 副本。但红 checks 会阻塞
ruleset（`verify` + `governance` 是 main 的必需检查），于是合并要么被拦、要么有人学会
「有一条 governance 红是正常的」——后者比前者更贵。

## Decision

**在全零哨兵与解析失败之间划一条线，只对后者 fail-closed。**

1. `ZERO_SHA_RE = /^0+$/` 独立成常量；`resolveCommit()` 仍对全零返回 `null`（它确实没有提交可比）。
2. `main()` 在调用 `resolveCommit()` **之前**先判全零：命中则输出
   `skipped（no-base-sentinel）… 由 pull_request 事件以 base.sha 复核` 并 `exit 0`。
3. 「给了 ref 却解析不出」（浅克隆 / 坏 ref / 拼错）维持 `exit 2` fail-closed 不变。
4. 自检 `check-independent-test-evidence.spec.sh` 的边界样本随之改写：
   - **N1**（原：全零 base ⇒ exit 2）改为 **全零 base ⇒ `exit 0` 且输出含 `no-base-sentinel`**；
   - 新增 **N1b**：`--base deadbeef…`（给了 ref、解析不出）⇒ 仍 `exit 2`。
5. `AGENTS.md` 的门禁速查同步该口径（原句「2 无法判定（base 解析失败）」会把两者混为一谈）。

为什么不是「在 workflow 里判 `github.event.before == '0000…'` 再跳过该 step」：那样 CI 日志里
只剩一句 workflow 层面的 `if`，判定逻辑散进 YAML，**本地无法复现、spec 无法覆盖**。哨兵语义
是门本体的知识，就该住在门里。

## Alternatives considered

- **新分支首推时不跑该 step（workflow 层 `if:` 跳过）**：改动更小，但把判据搬进 YAML ⇒ 本地跑
  `npm run contract:test-evidence` 时会走另一条路径，「CI 跑的和本地跑的不是同一个函数」正是本仓
  反复收敛要消灭的形态；且 skip 无法被 `spec.sh` 的样本锁住（YAML 条件没有正负样本可加）。
- **把全零 base 回填成默认分支（`origin/main`）再判**：新分支首推时那批提交可能远超 feature 范围
  （整条主分支历史都进 `base..head`），会把「历史上某条 dev-team 提交缺证据」的旧账算到本次推送
  头上 ⇒ 变成更难解释的假红；且需要 workflow 侧先 `fetch` 默认分支，门禁多出网络副作用。
- **首推全零 base 直接判 pass（不区分解析失败）**：会把「浅克隆导致 base 缺失」这个真实风险一并
  放行——`2026-11-25-independent-test-evidence-gate` 特意为此立了 fail-closed，不能为了消噪把它
  拆掉。故只拆「确定性事实（无前序）」与「异常（解析不出）」两类，前者跳过、后者仍红。
- **把 `--base` 改为可空并默认 `HEAD~1`**：`main()` 里 `args.base || head~1` 已有该兜底，但
  workflow 显式传了全零串（非空）⇒ 兜底不触发。改成「全零等价于未传」等于让 workflow 的
  `push` 分支静默退化为「只看最后一个提交」，判定范围被无声缩小，比显式跳过更危险。

## 证据

- `bash scripts/check-independent-test-evidence.spec.sh` → `PASS P1–P7 + N1/N1b/N2`（exit 0），
  输出含 `N1 全零 base 哨兵 ⇒ skipping(no-base-sentinel) exit 0` 与
  `N1b 坏 ref 时 fail-closed（exit 2，不伪装通过）`。
- 手工：`node scripts/check-independent-test-evidence.mjs --base 000…0 --head HEAD` → exit 0；
  `--base deadbeef…` → exit 2。
- 行为不降级：`--base origin/main` 走原判定路径（`skipped（no-dev-team-marker）`，exit 0）。
