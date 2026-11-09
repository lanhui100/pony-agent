# Agent Note: trace 审核 harness 断言脚本参数契约统一（F8）

Status: implemented

> 代码已落地；重冻结待用户批准，见文末「未决」。

## Context

r2-dim1 attempt 2 的补遗（F5 导出重试 + F7 批量 transport + 写域扩展）批准后，首次跑
`verdict.sh` 得到结构性失败：

```
== step 1: assert dim1   → assert=0（L1 0/9、L2 0/23、L3 0/23、L4 0/9 全绿）
== step 2: ratchet dim6  → FAIL 未知参数: --arm-tag
                            MISSING dim6（证据缺失 ≠ PASS）
verdict: UNKNOWN (evidence missing) exit=2
```

同一组参数**手工直调** `ratchet.sh`（不传 `--arm-tag`）→ `regression=0 missing=0 exit=0`，
`dim6.sh` 三条断言全 PASS（A1 mismatch=0、A2 release_null=0、A3 conflicting=0）。

⇒ 断言无误，缺陷在**调用方与被调用方的参数契约**：

| 位置 | 行为 |
| :-- | :-- |
| `ratchet.sh` | 对**所有**棘轮条目统一透传 `--arm-tag`（attempt 2 为修 L3 静默命中 0 行而加） |
| `verdict.sh` | 把 `--arm-tag` 传给 `ratchet.sh` |
| `assertions/dim1.sh` | 接受 ✅ |
| `assertions/dim6.sh` | 不接受 ⇒ `未知参数` exit=2 |

`dim6` 是棘轮集里唯一的已通过维度，因此只要它在，**任何后续维度的 verdict 恒为 UNKNOWN
exit=2，loop 结构性无法到达 PASS**。该缺陷此前未暴露，因为 attempt 1 的 verdict 在 step 1
就 FAIL，从未走到棘轮这一步。

## Decision

1. `assertions/dim6.sh` 参数解析新增 `--arm-tag) shift 2 ;;`（**接受并忽略**：dim6 的
   A1/A2/A3 按 metrics 全量判定，不做逐臂过滤，该参数在此语义上无需求）。
2. `selftest.sh` 新增「断言脚本参数契约」段：遍历 `assertions/dim*.sh`，用「缺 `--arm-dir`
   必然失败」区分「参数被接受」与「未知参数」，钉住契约防回归。自测 34 → **36 条，全绿**。
3. 不动任何阈值、corpus、断言判据代码。

「只加参数解析」由哈希等式机械证明：按文本精确剔除新增 7 行后重算 sha256 =
`60f68483a99f25d5928d28c64c6e6034ba576c4f83d66371e7d4ab0b87b97825`，与冻结值逐字节一致。

## Alternatives considered

- **`ratchet.sh` 按脚本 `--help` 探测是否支持该参数再决定透传**：隐式能力协商，探测失败即
  静默退化——正是本轮「静默命中 0 行」的同类缺陷形态；且 `ratchet.sh` 同样在冻结集内，
  改它同样要重冻结。否决。
- **`ratchet.sh` 只对 `dim1` 透传 `--arm-tag`**：硬编码维度分支，新维度必忘，同样是静默退化。
  否决。
- **`verdict.sh` 加 `--no-ratchet` 跑本维度裁决**：跳过棘轮等于放弃「历史维度不得回归」的
  门禁，把 UNKNOWN 换成假 PASS。否决。
- **把 `dim6.sh` 回滚、改为在 `verdict.sh` 里对 dim6 特判**：把契约歧义从被调用方挪到调用方，
  且 `verdict.sh` 在冻结集内。否决。

三个可行选项都要动冻结集文件，故**无论选哪个都必须重冻结**；选「被调用方接受参数」的理由是
它消除契约歧义本身，而不是把歧义挪进探测/特判逻辑。

## 影响与未决

- 冻结集 `scripts_assertions_dim6.sh` 由 `60f68483…` 变为 `93d7b4ec…`；其余 13 项逐键比对未变。
- **未决**：`state.py check-freeze` 现报 exit=4（冻结漂移），按「批准时冻结、事后改动须新提案
  重审」纪律，重冻结须用户批准 `docs/audit/rounds/r2-dim1/proposals/r2-dim1.attempt2-harness-fix.md`。
