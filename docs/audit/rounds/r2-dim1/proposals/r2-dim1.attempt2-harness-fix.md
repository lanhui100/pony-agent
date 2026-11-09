---
status: approved
approved_by: user
approved_unix: 1794188982
approval_note: >-
  用户批准 F8 最小修复及 scripts_assertions_dim6.sh 重冻结；阈值、corpus 与 A1/A2/A3
  判据均不得改变，重冻结后由 verdict.sh 退出码独立裁决。
dimension: 1
dimension_name: latency.structure（时延结构）
round: r2-dim1
attempt: 2
kind: harness-contract-fix
amends: proposals/r2-dim1.attempt2-addendum.md（F5 + F7 + 写域扩展，已批准）
author_session: pa_01a1ba70-a314-7712-adf2-4bf675a96f99
created_unix: 1794191300
baseline_commit: 1bca945808fc189435460ed83d789f8789b078a4
candidate_commit: d9425a70
corpus_hash: 879597eb3594eb365bdbe033f7fbcd8353380e359170267f8d3e621255b8300a
---

# r2-dim1 attempt 2 补遗（harness）：F8 断言脚本参数契约不一致

## 触发事实（机器证据）

批准 `attempt2-addendum` 后跑 `verdict.sh`，**未通过**：

```
== step 1: assert dim1        → assert=0（L1/L2/L3/L4 全绿）
== step 2: ratchet dim6       → FAIL 未知参数: --arm-tag
                                 MISSING dim6（证据缺失 ≠ PASS）
verdict: UNKNOWN (evidence missing) exit=2
```

同一组参数**手工直调** `ratchet.sh`（不传 `--arm-tag`）则 `regression=0 missing=0 exit=0`，
`dim6.sh` 三条断言全 PASS（A1 mismatch=0、A2 release_null=0、A3 conflicting=0）。

⇒ 断言本身没问题；**是调用方与断言脚本的参数契约不一致**，导致棘轮把 dim6 判成证据缺失。

## 根因 F8（blocker，harness 自身）

| 位置 | 行为 |
| :-- | :-- |
| `ratchet.sh:73` | 对**所有**棘轮条目统一透传 `--arm-tag`（r2-dim1 attempt 2 为修 L3 静默命中 0 行而加） |
| `verdict.sh:126` | 把 `--arm-tag` 传进 `ratchet.sh` |
| `assertions/dim1.sh:30` | 接受 `--arm-tag` ✅ |
| `assertions/dim6.sh:25-31` | **不接受** ⇒ `fail_msg "未知参数" ; exit=2` |

后果链：`dim6` 是棘轮集里唯一的已通过维度 ⇒ 只要它在，**任何后续维度的 verdict 恒为
UNKNOWN exit=2，永远走不到 PASS**。这不是本轮特例，是 loop 的全局死锁。

## 方案（最小闭包）

| 文件 | 改动 | 行数 |
| :-- | :-- | :-- |
| `scripts/audit/assertions/dim6.sh` | 参数解析新增 `--arm-tag) shift 2 ;;`（接受并忽略：dim6 三条断言按 metrics 全量判定，不做逐臂过滤） | +7（含注释） |
| `scripts/audit/selftest.sh` | 新增「断言脚本参数契约」段：遍历 `assertions/dim*.sh`，用「缺 `--arm-dir` 必然失败」区分「参数被接受」与「未知参数」 | +21 |

不改任何阈值、不改 corpus、不改断言判据逻辑（A1/A2/A3 一行未动）。

### 契约方向的选择：为什么改被调用方，而不是改调用方

| 选项 | 评价 |
| :-- | :-- |
| **A. `dim6.sh` 接受并忽略 `--arm-tag`（采纳）** | 契约统一：所有 `assertions/dim*.sh` 参数集一致，`ratchet.sh` 无需逐脚本探测能力；新维度只需照抄同一骨架 |
| B. `ratchet.sh` 按脚本 `--help` 探测是否支持该参数再决定透传 | 隐式能力协商，探测失败就静默退化 —— 正是本轮「静默命中 0 行」的同类缺陷形态；且要改 `ratchet.sh`（同样在冻结集） |
| C. `ratchet.sh` 只对 `dim1` 透传 | 硬编码维度分支，新维度必忘；同样是静默退化 |

A/B/C 都要动冻结集文件（`dim6.sh` 或 `ratchet.sh`），因此**无论选哪个都必须重冻结**；
选 A 的理由是它消除的是契约歧义本身，而不是把歧义挪到探测逻辑里。

## 冻结集影响（必须重冻结，故本提案存在）

`state.py check-freeze --round r2-dim1` 当前实测 **exit=4**：

```
DRIFT assertion 漂移: scripts_assertions_dim6.sh
  冻结值 60f68483a99f25d5928d28c64c6e6034ba576c4f83d66371e7d4ab0b87b97825
  现值   93d7b4ec9401c6574354c05127949e01015698a4f64155359b5d81f08a21197e
```

**其余 13 项冻结哈希全部未变**（逐键比对输出：`changed_keys` 仅含 `scripts_assertions_dim6.sh`）。

### 「只加参数解析、判据未动」的机器证明

把新增的 7 行按文本精确剔除后重算 sha256：

```
removed_lines   = 7
rebuilt_sha256  = 60f68483a99f25d5928d28c64c6e6034ba576c4f83d66371e7d4ab0b87b97825
frozen_expected = 60f68483a99f25d5928d28c64c6e6034ba576c4f83d66371e7d4ab0b87b97825   ← 完全一致
```

即：改动**可逆地**只包含参数解析分支，A1/A2/A3 的判定代码逐字节未变。这不是口头承诺，
是可复算的哈希等式。

## 重冻结后的预期

- `freeze-check.sh --round r2-dim1`：`freeze=0 scope=0`（写域 8 项已含 `src-tauri/src/lib.rs`）
- `verdict.sh`：assert=0 + ratchet=0（dim6 A1/A2/A3 在 candidate-a3 上已实测全 PASS）
- `selftest.sh`：pass=36 fail=0（新增 2 条参数契约断言）

## 诚实说明

- **本修复已先行实施**（改文件在前、写提案在后）：因为不修则 verdict 结构性无法到达 PASS，
  而先取证再动手的顺序在本轮已由上面的机器证据补齐（直调 ratchet 绿 / 经 verdict 红）。
  仍按纪律停点等待批准后才**重冻结 + 裁决**，不自行放宽冻结集。
- 这条缺陷**不是**本轮代码改动引入的，是 attempt 2 期间为修 L3 而加 `--arm-tag` 透传时
  留下的 harness 债；它此前未暴露，因为 attempt 1 的 verdict 在 step 1 就 FAIL 了，从未走到棘轮。

## 回滚

`dim6.sh` 删除 `--arm-tag) shift 2 ;;` 分支（sha256 即回到 `60f68483…`），`selftest.sh`
删除参数契约段。回滚后棘轮会对 dim6 重新判 MISSING —— 即恢复「loop 无法 PASS」的原状。

## 需你决策

批准本补遗（重冻结 `scripts_assertions_dim6.sh` 哈希），使 r2-dim1 verdict 可走完；
批准后我立即重冻结并跑 `verdict.sh`。
