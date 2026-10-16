# wave-18c 闭合复审 findings 逐条裁定表（T7 / L2-P 协议特派员）

- 任务：`task-7`（wave-18c 契约升版 r4）
- 契约基线：`.dev-team/contract-matrix-wave18.md` revision **`wave18-r4`**
- 证据源（闭合复审）：
  1. `.dev-team/review-wave18b-scanner.json`（verdict **FAIL**，4 项 findings：3 项 NOT-CLOSED + 1 项新误报回归）
  2. `.dev-team/review-wave18b-contract.json`（verdict **FAIL**，2 项 findings：R6 口径矛盾 + 看板第二次假断言）
  3. 旁证：`.dev-team/review-wave18b-l3p.json`（verdict **PASS**，Q1–Q5 全 PASS，无 findings）
- **合计 6 项 findings**（scanner 4 + contract 2）。
- 裁定口径：**FIX 5 项 · CLOSED（实质闭合 + 措辞纠正）1 项**。
- 责任任务（共享任务板实况）：`task-8` = T8 红相（`blocked_by: task-7`）· `task-9` = T9 实施（`blocked_by: task-8`）。

---

## 1. 裁定总表（6/6 覆盖）

| # | 来源 finding | 位置 | 缺陷摘要 | 裁定 | 责任任务 | 契约落点 |
|---|---|---|---|---|---|---|
| 1 | `review-wave18b-scanner` #1 | `scripts/check-secrets.mjs:128` | **NEW-FP（r3-R1 引入的新误报回归）**：`if (f.credWord) return false;` 在四处豁免**之前**、且 `CRED_WORD_RE` 对**整行**求值 → 合法摘要收据只要同行（路径/文件名/字段名/说明文字）含 `secret`/`token`/`credential` 等词即被判泄漏，违反"零误报为绑定约束" | **FIX** | **task-8**（红相）+ **task-9**（实施） | **§4.2 C4-R1′** + §4.3 + §4.8 FP1–FP6 |
| 2 | `review-wave18b-scanner` #2 | `:131`（`f.prefixHex === m`）、`:132`（`f.cellHex === m`） | **NEW-FN / 冻结语义偏离**：豁免 2/3 按**值**锚定而非**位置**锚定 → `<H>  <H>` 与 `\| <H> \| <H> \|` = **0 命中**，违反 §4.3-R2「一行内其他位置的 64-hex 不继承豁免」 | **FIX** | **task-8** + **task-9** | **§4.2 C4-R2′** + §4.3 + §4.7 G13–G16 |
| 3 | `review-wave18b-scanner` #3 | `:125`（`const line = context.split("\n").pop();`，调用点 `:195`） | **NEW-HIGH-DoS（残余二次方）**：`context.split("\n")` 每 match 执行一次 → 仍为 O(matches × 行长)；实测 1MB=248.5ms / 2.6MB=1894.7ms / 5MB=34841.9ms / 10MB=176568ms；`--staged`/`--pre-push` 钩子无 1MB 上限（仅 `--all` 有）→ 10MB 单行文件可停摆钩子 176s；§4.6 不变量未达成且代码注释（`:49`）失真 | **FIX** | **task-8** + **task-9** | **§4.6（r4 修订判据）** + §4.2 C4-R3′ |
| 4 | `review-wave18b-scanner` #4 | `:22`（`CRED_WORD_RE`） | **NEW-FN（残余漏报）+ §4.5 过头表述**：正则不含空格分隔形态 → `\| client token \| <H> \|` 与 `\| token \| <H> \|` = **0 命中**；§4.5 却声称"r3 已关闭该缺口" | **FIX** | **task-8** + **task-9** | **§4.2/§4.3 C4-R4** + **§4.5 重写（B1–B5 逐条登记）** + §4.7 G7–G12 |
| 5 | `review-wave18b-contract` #1 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:395` | R6 要求"未满足条件时以 `ignored` 如实上报"，实测 skip 分支仍报 `ok`、harness `0 ignored`；但四项**实质**要求（拆两段 / 恒跑真断言 / skip 分支自带真断言 / 显式 `[skip]` 打印）**均已达成** | **CLOSED（实质闭合）＋ 措辞纠正** | **—**（无需实施；契约口径由 task-7 改写） | **§9.2 R6（r4 重写）** + **§11 M13（→ CLOSED）** |
| 6 | `review-wave18b-contract` #2 | `management/task-system/00_DASHBOARD.md:23` | **R7 第二次假断言**：修正文本把"全仓零命中"改为代码域限定（该分句为真），但新增存在性分句「仅见于任务卡 AC 文案 `03_TASKS/PA-110-...:26`」在同一全仓 grep 下为假 | **FIX（二次修正）** | **task-8**（红相）+ **task-9**（实施，其写域含该文件） | **§9.2 R7（r4 重写 + 三条可复现验收判据）** |

---

## 2. 各项反例证据（本特派员**独立复现**，非引用）

> 探针 `/tmp/w18c-proto-probe.mjs`（r4 候选实现，只读、未写仓库）与内联 `node -e`（对**现行 r3 代码**求值）。64-hex 一律由 `"c".repeat(64)` 等运行时构造，探针文件本身不含完整字面量。

### 2.1 finding#1（新误报回归）— 现行 r3 实测

| 输入 | r3 实测命中 | r4 要求 | 裁定依据 |
|---|---|---|---|
| `<H>  scripts/rotate-secret.sh` | **1**（误报） | 0 | `review-wave18b-scanner` #1 反例 A |
| `<H>  scripts/rotate-token.sh.bak`（对照，文件名无凭证词） | 0 | 0 | 同 finding 反例 A 的 control |
| `{"file":"secrets.json","sha256":"<H>"}` | **1** | 0 | 同 finding 反例 B（`old=0 new=1`） |
| `- 密钥指纹(secret fingerprint): <H>` | **1** | 0 | 同 finding 反例 B |
| `<H>  config/credential-store.json` | **1** | 0 | 同 finding 反例 B（new-only hits） |
| `<H>  scripts/reset-password.sh` | **1** | 0 | 同 finding 反例 B（new-only hits） |
| `` \| `<H>` \| sha256 of SECRET_RULES sample \| `` | **1** | 0 | 同 finding 反例 B（反引号豁免被击穿） |

**r4 候选实现实测**：FP1–FP6 **全部 0 命中**（`/tmp/w18c-proto-probe.mjs fp`，14/14 PASS，Exit 0）。

### 2.2 finding#2（值锚定 vs 位置锚定）— 现行 r3 实测

| 输入 | r3 实测 | r4 要求 | 说明 |
|---|---|---|---|
| `H  H`（同值重复，sha256sum 形态） | **0** | ≥1 | 值锚定使重复值继承豁免 |
| `\| H \| H \|`（同值重复，表格形态） | **0** | ≥1 | 同上 |
| `H  H2`（异值） | 1 | ≥1 | 对照组 |
| `\| H \| H2 \|`（异值） | 1 | ≥1 | 对照组 |

**r4 候选实现实测**：R2a–R2d **全部 ≥1**（`regress` 探针 16/16 PASS，Exit 0）。

### 2.3 finding#3（残余二次方）— 现行 r3 实测（本特派员独立复现）

| 规模（单行，distinct 64-hex 空格连接） | r3 实测 | 评审路实测 | r4 判据 | r4 候选实测 |
|---|---|---|---|---|
| 1MB（n=15000，974,999 B） | **244.2 ms** | 248.5 ms | — | — |
| 2.6MB（n=40000，2,599,999 B） | **2560.0 ms** | 1894.7 ms | — | — |
| 5MB（n=77000，5,004,999 B） | **27837.9 ms** | 34,841.9 ms | **≤ 2000 ms** | **99.6 ms** ✅ |
| 10MB（n=154000，10,009,999 B） | （未跑，评审 176,568 ms） | 176,568.0 ms | **≤ 5000 ms** | **161.1 ms** ✅ |

- 根因确认：`context.split("\n").pop()` 每 match 一次（评审微基准 0.6455 ms/call × 77000 matches ≈ 50 s，与实测同量级）。
- 可达性：`--all` 跳过 >1MB（`check-secrets.mjs:306`）→ CI 不阻塞；`--staged`/`--pre-push` **无上限** → 钩子可被单个 10MB 单行文件停摆。

### 2.4 finding#4（空格形态残余漏报）— 现行 r3 实测

| 输入 | r3 实测 | r4 要求 |
|---|---|---|
| `\| client token \| <H> \|` | **0** | ≥1 |
| `\| token \| <H> \|` | **0** | ≥1 |
| `\| access token \| <H> \|` | **0** | ≥1 |
| `\| api key \| <H> \|` | **0** | ≥1 |
| `\| ClientToken \| <H> \|` | 1 | ≥1（不得回退） |
| `\| client-token \| <H> \|` | 1 | ≥1（不得回退） |

**r4 候选实现实测**：R4a–R4f **全部 ≥1**（`regress` 16/16 PASS）。

### 2.5 finding#5（R6 口径矛盾）— 事实与裁定

- **矛盾**：r3 冻结「M13/R6 须以 `ignored` 如实上报」 ↔ `task-5` §8「不得使用 `#[ignore]`」。
- **Lead 裁定（wave18-r4）**：以**实质**为准 —— skip 分支必须 ① 自带真断言、② 打印显式 `[skip]` 标记、③ 在 `.dev-team/red-purity-wave18b.json` 登记 harness 级跳过；**不要求** `#[ignore]`，**不要求** harness `ignored` 计数。
- **本特派员复核现行实现**（`crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs`）：
  - 恒跑真断言段存在：`test_upload_trace_direct_sender_is_fail_closed_without_credentials`；
  - live 用例 skip 分支含真断言 `assert_ne!(live_opt_in.as_deref(), Some("1"), ...)`，并 `eprintln!("[skip] ...")`；
  - 文件内自述"不使用 `#[ignore]`（task-5 明令）"；
  - 跳过事实登记于 `.dev-team/red-purity-wave18b.json`。
  - ⇒ **三项实质要求全部达成** → 裁定 **CLOSED**；复审观测到的 `0 ignored` / `ok` 不再构成缺陷。
- 契约侧动作：task-7 已按新口径重写 §9.2 R6 与 §11 M13（M13 状态由 "P0（r3 新增）" 改为 "**CLOSED（实质闭合）**"）。

### 2.6 finding#6（R7 第二次假断言）— 本特派员独立复现（r4 时点）

```
$ git grep -n MAX_SUBAGENT_DEPTH | wc -l
10
$ git grep -n MAX_SUBAGENT_DEPTH -- '*.rs' '*.ts' ; echo EXIT=$?
EXIT=1        # 0 命中（代码域分句为真）
```

命中集合（10 行）：

| # | 位置 | 类别 |
|---|---|---|
| 1 | `.dev-team/contract-matrix-wave18.md:506` | `.dev-team/` 契约档案 |
| 2 | `.dev-team/review-l3-contract-wave18.json:20` | `.dev-team/` 审查档案 |
| 3–4 | `.dev-team/review-l3-security-wave18.json:7`、`:8` | `.dev-team/` 审查档案 |
| 5–6 | `.dev-team/review-wave18b-contract.json:13`、`:14` | `.dev-team/` 审查档案 |
| 7–8 | `.dev-team/wave18b-adjudication.md:22`、`:64` | `.dev-team/` 裁定档案 |
| 9 | `management/task-system/00_DASHBOARD.md:23` | **该断言行自身** |
| 10 | `management/task-system/03_TASKS/PA-110-agent-teams-and-subagent-tools.md:26` | 任务卡 AC 文案 |

- 看板现文本声称「零命中，**仅见于**任务卡 AC 文案 `03_TASKS/PA-110-...:26`」→ **后半句为假**（除任务卡外另有 8 处非代码命中 + 该行自身）。
- **命中数是时变量**（任何引用该标识符的新文档都会增加命中）：评审路测得 **8**，本特派员 r4 时点测得 **10**（新增 2 处来自 r3 的契约/裁定档案）。故 r4 **冻结的验收判据是"集合一致性"而非固定数字**（§9.2 R7 判据 1–3）。
- **r4 冻结表述**（推荐）：全仓**代码**（`*.rs`/`*.ts`）零命中；非代码命中仅见于**本行自身**、`03_TASKS/PA-110-...:26` 的任务卡 AC 文案，以及 `.dev-team/` 下的审查与契约档案。
- **r4 冻结验收判据**：① `git grep -n MAX_SUBAGENT_DEPTH -- '*.rs' '*.ts'` => EXIT=1；② `git grep -n MAX_SUBAGENT_DEPTH | grep -v '^\.dev-team/'` => **恰好 2 行**（该行自身 + 任务卡 :26）；③ 文本不得出现穷举式"仅见于…"清单。

---

## 3. 计数核对

| 裁定 | 数量 | 对应 finding |
|---|---|---|
| **FIX** | **5** | scanner#1、#2、#3、#4 + contract#2 |
| **CLOSED（实质闭合 + 措辞纠正）** | **1** | contract#1（R6 口径矛盾） |
| NOT-CLOSED（未修，无新契约） | **0** | — |
| **findings 总数** | **6** | 5 + 1 = 6 ✅ |

---

## 4. r4 契约变更与 finding 的对应关系

| finding | r4 契约落点 | 交付给 |
|---|---|---|
| scanner#1 | §4.2 C4-R1′（位置敏感 + `!benignBefore` 细化，见 §12.4 显式披露）、§4.3、§4.8 FP1–FP6 | task-8 红相 / task-9 实施 |
| scanner#2 | §4.2 C4-R2′（`matchIndex === slotStart`）、§4.3、§4.7 G13–G16 | 同上 |
| scanner#3 | §4.6（判据 5MB ≤2000ms / 10MB ≤5000ms + `per_match_context_split_forbidden`）、§4.2 C4-R3′（`ctxLineFacts` 按 `lineNumber` 记忆化）、§7.3 约束 5（新增 `match.index`/`lineNum` 实参） | 同上 |
| scanner#4 | §4.2/§4.3 C4-R4（正则扩面）、§4.5 重写（B1–B5）、§4.7 G7–G12 | 同上 |
| contract#1 | §9.2 R6（口径重写）、§11 M13（→ CLOSED）、§11 头部例外条款 | 契约侧已闭合，无实施项 |
| contract#2 | §9.2 R7（二次修正 + 三条判据） | task-8 红相 / task-9 实施 |

**NFR 基线同步**（`.dev-team/nfr-baseline-wave18.json`）：`wave18b_scanner_linearity` 新增 `per_match_context_split_forbidden`、`line_cache_key="lineNumber"`、`acceptance_max_ms_single_line_5mb=2000`、`acceptance_max_ms_single_line_10mb=5000`、r3 实测与 r4 候选实测字段；`python3 .dev-team/nfr-lint.py --instance .dev-team/nfr-baseline-wave18.json` → **RESULT: PASS，EXIT=0**。

---

## 5. 执行顺序（依赖约束）

```
task-7（本文件，r4 契约冻结）  ✅
   └─ task-8（红相：R1′/R2′/R3′/R4 + R7 的失败测试 + 变异敏感收据）
         └─ task-9（实施：scripts/check-secrets.mjs + management/.../00_DASHBOARD.md）
               └─ 闭合复审（scanner / contract / L3-P）+ Lead 终审
不阻塞：R6（本文件判 CLOSED，无需实施）；R1/R2/R3/R4(DEBT-W18-R4) 为欠账登记
```
