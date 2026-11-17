# Join Barrier 复审 r2（wave `acp-delegate-p0p2`，只读）

- 分支/工作树：`fix/acp-delegate-p0p2` @ `/home/dm/pony-agent-worktrees/acp-delegate-p0p2`；HEAD = `5a914c4c`（复审对象）
- 复审触因：上一轮 Join **FAIL**（1 BLOCKING + 6 ADVISORY，`reviewed_commit=aaec5ae3`），本轮只复核该 BLOCKING 与 6 条 ADVISORY 的修复面 + 收据抽验
- 只读声明：**未改** src/测试/契约/证据，**未做** git 写；仅用 `cargo-lane.sh` 跑 2 个契约套件 + 1 个持久化单例 + 只读 grep/sed + 1 次 CLI 现场探针（隔离 `HOME`/`XDG_CONFIG_HOME`，产物落 `/tmp`）
- 机读收据：`.dev-team/review-join-acp-delegate-r2.json`（`dev-review-lint.py` **Exit 0**，见 §4）

## 0. 裁决

**verdict = PASS**：0 BLOCKING / 7 ADVISORY。

上一轮唯一 BLOCKING（模型可见描述仍称「截断轮不返回 `finalText`」）**已消除且断言已收紧**：`schema.rs:1145` 现明确「成功轮与截断轮（`max_tokens`/`max_turn_requests`）都回传 `finalText`（超 32000 字符保留前缀并置 `finalTextTruncated=true`；截断轮另带 `truncated=true`）；仅拒绝/取消/错误不返回 `finalText`」，与 `acp/session.rs:1432` 的 `Ok | Truncated` 合流、`:1522-1526` 的键面、`:1541` 失败面不产正文**逐条一致**；`T-ACP-05` 的逃逸 OR 子句已删除并改为子句级严格断言。

残余 7 条均为 ADVISORY（文档/台账/描述精度/测试口径），不构成交付否决。

---

## 1. 逐项复核表（file:line + 命令/PASS-FAIL）

| # | 复核项 | 关键证据（file:line） | 命令 / 现场 | 判定 |
|---|---|---|---|---|
| 1 | **BLOCKING**：`schema.rs` 描述 vs `session.rs` 键面 | `agent/tools/schema.rs:1141-1145`（注释+描述）；`acp/session.rs:1303-1310`（max_tokens/max_turn_requests→`Truncated`）、`:1431-1438`（`Ok\|Truncated`→同一构造器）、`:1522`（恒插 `finalText`）、`:1523-1526`（`finalTextTruncated = final_text.truncated \|\| stop_reason_limit_hit`）、`:1541-1557`（失败面**无** `finalText` 键）；`acp/result.rs:25`（`32_000`）、`:81-92`（`usage{used,size,cost{amount,currency}}`） | 三处逐键比对见 §2-1；`grep -rn '不返回 \`finalText\`' crates/pony-agent-core/src` 仅剩 `schema.rs:1145` 的「仅拒绝/取消/错误不返回」子句 | **PASS** |
| 2 | A1：`T-ACP-05` 删逃逸子句 + 严格断言 | `tests/acp_delegate_p0p2_contract.rs:1496`（用例）、`:1510-1518`（两键声明）、`:1523`（取截断子句）、`:1529-1539`（子句含 `finalText` / 不含`不返回` / 含 `finalTextTruncated`） | `git show 5a914c4c -- .../acp_delegate_p0p2_contract.rs` 显示 OR 逃逸子句被整段替换；套件 15 passed | **PASS** |
| 3 | A3/A4：R3 stdout 负向断言 + c0_4a2 非空正文截断 | `tests/acp_cli_resume_and_epipe.rs:444-452`（stdout 不得含 `message`+`trace_id` 行）；`tests/acp_delegate_p0p2_contract.rs:508-553`（`c0_4a2`）、`:124-152`（`TRUNC_CHUNK_AGENT_SCRIPT` 先发正文再 `max_tokens`）、`:230`（`fake-trunc-chunk` 装配） | 两套件真跑 15+5 passed（§4）；`c0_4a2` 断言 `finalText == "PARTIAL-BODY-BEFORE-MAX-TOKENS\|"` 且非空 | **PASS** |
| 4 | A2：`acp_early_log_redirection` 位置 + `--help --output-format jsonl` 口径 | `bin/cli/acp.rs:45-67`（预解析探测）、`:70`（**parse 之前**调用）、`:72`（`parse_acp_cli_args`）、`:187-189`→`:566-588`（help 仍 `println!` 到 stdout）；`acp_usage_error`→stderr（`:371-374`） | 现场：`ponycli acp --help --output-format jsonl` → Exit 0、stdout **1498B**（21 行 help 文本）、stderr **0B**（无结构化日志泄漏） | **PASS（改道位置）/ ADVISORY（help 文本仍在 stdout）** |
| 5 | A6：证据日志已刷新为修复后最终运行 | `docs/release/evidence/acp-delegate/all-acp-final.log`（`:116` `t_acp_05_… ok`、`:134` `c0_4a2_… ok`、`:146` 15 passed、全档 19 个 `test result: ok`、0 `FAILED`）；`final-lib.log`（`:3410` `1614 passed; 0 failed`） | `diff -q` 两个证据文件与 `/tmp/final2-acp.log`、`/tmp/final2-lib.log` → **IDENTICAL**；套件二进制 hash `1f31f6a165f2e780` 与本轮真跑一致（证明非旧档） | **PASS** |
| 6 | A5：残余 ADVISORY 已登记 | `.dev-team/state.json:197-228`（`residual_advisories`） | 实测 ids = `B239,B240,A2,A3,A4,F2`（六项齐全） | **PASS（六项在册）/ ADVISORY（台账保真度）** |
| 7 | 收据抽验 | `/tmp/final2-lib.log:3410`（1614/0）、`/tmp/final2-acp.log`（19 个 ok、0 FAILED） | 本轮真跑 `--test acp_delegate_p0p2_contract --test acp_cli_resume_and_epipe` → **`15 passed` + `5 passed`，Exit 0** | **PASS** |

---

## 2. 逐项详述

### 2-1 BLOCKING（上轮唯一否决项）：已修复，描述与实现逐键一致

| 描述声称（`schema.rs:1145`） | 实现事实 | 一致 |
|---|---|---|
| 「成功轮与截断轮（stopReason=`max_tokens`/`max_turn_requests`）都回传 `finalText`」 | `session.rs:1306` `MaxTokens\|MaxTurnRequests → Truncated`；`:1432` `Ok \| Truncated` 同一构造器 | ✅ |
| 「超过 32000 字符只保留前缀并置 `finalTextTruncated=true`」 | `result.rs:25` `ACP_DELEGATE_FINAL_TEXT_MAX_CHARS = 32_000`；`session.rs:1523-1526` `finalTextTruncated = final_text.truncated \|\| limit_hit` | ✅ |
| 「截断轮另带 `truncated=true`」 | `session.rs:1517` `truncated = stop_reason_limit_hit` | ✅ |
| 「并含可选 `usage`（used/size/cost）」 | `result.rs:81-92` 顶层 `used`/`size`、`cost{amount,currency}`；`:1531` 无 `usage_update` 则键缺席 | ✅ |
| 「无正文时 `finalText` 为空串且 summary 明示未返回文本」 | `session.rs:1522` 恒插空串；`:1496-1499` summary「未返回文本正文」 | ✅ |
| 「仅拒绝/取消/错误不返回 `finalText`，须按 `error.code` 处置」 | `session.rs:1439-1451` Refused/Cancelled → `build_failure_result`；`:1541-1557` 失败面键面无 `finalText`/`finalTextTruncated` | ✅ |

旧文「非 end_turn 归约（**截断**/拒绝/取消/错误）不返回 `finalText`」在工作树中已无残留（`grep` 命中仅测试断言文本与历史 review 产物）。C0.5「声明…与实际行为一致」恢复成立。

口径精度（ADVISORY A-7，非矛盾）：描述把 `finalTextTruncated=true` 明确绑定在「超过 32000 字符」条件，末尾以「截断轮另带 `truncated=true`」收尾；实现对 `max_tokens`/`max_turn_requests` 轮**无条件**置 `finalTextTruncated=true`（`session.rs:1523-1526`，短正文亦然）。语义方向一致（截断轮确为 true），但「截断轮 `finalTextTruncated` 亦为 true」这一组合未在描述里显式给出。

### 2-2 A1：断言已从「恒真逃逸」改为「子句级双向锁定」

- 旧：`!description.contains("不返回 \`finalText\`") || description.contains("非 end_turn")` —— 两串同时存在即恒真。
- 新（`:1523-1539`）：按 `；`/`。` 切子句，取首个含「截断」的子句，**无 OR 分支**地断言（a）含 `finalText`、（b）**不含**「不返回」、（c）含 `finalTextTruncated`。
- 该断言现对描述回归（把截断重新列入「不返回」集合）会真实变红 —— 与 BLOCKING 同源问题已被同一用例双向锁定。
- 残余口径（ADVISORY A-2）：`find()` 只取**第一个**含「截断」的子句；后续追加一条矛盾子句仍可逃逸。

### 2-3 A3/A4：负向断言与非空正文截断均已落地

- A3：`acp_cli_resume_and_epipe.rs:444-452` 在「stdout 必须有 `turn:delta`」（`:421`）之外新增负向断言：stdout 中凡同时带 `message` 与 `trace_id` 的对象即为泄漏诊断（与 `otel/log.rs:128-131` 的键面一致 ⇒ 判据可命中真实日志行）。改道回退到 stdout 时该断言必红。
- A4：`c0_4a2`（`:508`）用独立 fake agent `fake-trunc-chunk`（`:124-152` 先发 `agent_message_chunk`，再回 `stopReason:max_tokens`），断言 `finalText` **非空且逐字等于** chunk 正文、`finalTextTruncated=true`、`truncated=true`、无 `error` 对象 ⇒ 「截断轮带着非空正文成功回传」主路径已被覆盖。

### 2-4 A2：改道已前移到 parse/help 之前，但 help 文本仍走 stdout（ADVISORY 保留）

- 位置正确：`acp.rs:70` 在 `parse_acp_cli_args`（`:72`，其 `--help` 分支 `:187-189` → `print_acp_usage`）**之前**执行；预探测只认 `--output-format <fmt>` 相邻值，与正式解析器支持的形态一致（`:224-240`），非法值不提前判错、不改退出码 ⇒ 注释所称「必须在任何 stdout 输出之前改道」在**日志面**成立。
- 现场口径：`--help --output-format jsonl` 的 stdout 仍是 1498B help 文本（`print_acp_usage` 全 `println!`，`acp.rs:566-588`），stderr 0B。即「日志不再污染 stdout」达成，「`jsonl/sse: stdout = events only`（help 文本亦不出现）」**未**达成；`state.json` 已把 A2 登记为残余（owner `executor-a`），但契约矩阵未登记该豁免。归 ADVISORY A-1。

### 2-5 A6/A5：证据与台账

- 证据：`all-acp-final.log` 与 `final2-acp.log` 字节一致、`final-lib.log` 与 `final2-lib.log` 字节一致；ACP 档 19 个 `test result: ok`、0 `FAILED`、含新用例 `t_acp_05`/`c0_4a2`，且契约套件由 14→**15 passed** ⇒ 确为修复后运行，不再是旧档（上轮 A6 已闭环）。
- 台账：`residual_advisories` 六项齐全（B239/B240/A2/A3/A4/F2），每项带 `owner`；但（ADVISORY A-4）A3/A4 本轮**已修复**却仍列残余、六项均无到期/重跑验收字段（与 `meta-retrospective.md:9`「并在 state.json 记录 owner 与到期」不符）、复盘称「残余 12 条已登记」而实际仅 6 条摘要条目。

---

## 3. 残余 ADVISORY 清单（7 条，均带 file:line + 反例命令，见 JSON）

| id | 位置 | 内容 | 建议 |
|---|---|---|---|
| A-1 | `src/bin/cli/acp.rs:585`（`print_acp_usage` 全 `println!`，`:566-588`；调用点 `:187-189`） | jsonl/sse 下 `--help` 仍向 stdout 打 1498B help 文本（实测 stderr 0B），与 help 自述 `jsonl/sse: stdout = events only`（`:576`）口径不符；已登记残余 A2 但未在契约矩阵登记豁免 | 二选一收口：非 text 模式把 help 改 stderr，或在契约矩阵 C3.1 显式登记「`--help` 豁免 stdout 纯净」并更新 `state.json` 的 A2 描述（现文「--help 在改道前打印 stdout」已过时） |
| A-2 | `tests/acp_delegate_p0p2_contract.rs:1523` | `find()` 只检查**第一个**含「截断」的子句；日后追加「…；截断轮不返回 `finalText`」式矛盾子句仍可通过（已用同构描述实测逃逸） | 改 `filter(...).all(...)`：所有含「截断」的子句都必须满足三条件 |
| A-3 | `docs/architecture/acp-client-integration.md:168` | 文档写 `usage{used,size,costAmount?,costCurrency?}`，实现/测试为 `usage{used,size,cost{amount,currency}}`（`acp/result.rs:87-89`、`tests/acp_delegate_p0p2_contract.rs:648-655`）⇒ 文档键名与实际契约不符 | 文档改为 `cost{amount,currency}`（或注明 flatten 前形态） |
| A-4 | `.dev-team/state.json:197`、`.dev-team/meta-retrospective.md:9` | `residual_advisories` 保真度：A3/A4 已修复仍在册；六项仅 `id/owner/desc`（无到期、无重跑验收），与复盘「记录 owner 与到期」「12 条已登记」不符 | A3/A4 移出（或加 `status:"fixed"`）；补 `due`/`acceptance` 字段；复盘数字与台账对齐 |
| A-5 | `tests/acp_delegate_persistence_contract.rs:628` | C6.2 后半在 `created_at_ms IS NULL` 时仅 `println!` 空过（无断言）⇒「会话 createdAt = 创建时刻」在该形态下不可验；本轮实测命中 `Some` 分支（`--nocapture` 无 `[C6.2]` 输出），属潜在空过分支，仍未登记残余 | 登记为 residual（ACL：`created_at_ms` 必须非空再断言），或把 NULL 判为失败 |
| A-6 | `src/agent/tools/schema.rs:1145` | 描述把上限**硬编码**为「32000 字符」，与 `ACP_DELEGATE_FINAL_TEXT_MAX_CHARS=32_000`（`acp/result.rs:25`）无机器耦合；`T-ACP-05` 不断言数值 ⇒ 常量一改，模型可见契约静默漂移（C0.3「单一常量真源」在描述面失守） | 在 T-ACP-05 增断言：描述须含 `ACP_DELEGATE_FINAL_TEXT_MAX_CHARS` 的当前值（或由常量生成描述片段） |
| A-7 | `src/agent/tools/schema.rs:1145` | `finalTextTruncated=true` 在描述里绑定「>32000 字符」条件，而实现对 `max_tokens`/`max_turn_requests` 轮**无条件**置 true（`acp/session.rs:1523-1526`，短正文亦然）；「截断轮另带 `truncated=true`」未显式覆盖该组合 ⇒ 模型可读为「只有本地 32k 截断才置 `finalTextTruncated`」 | 补一句「截断轮 `finalTextTruncated` 亦为 true（正文不完整）」；T-ACP-05 可加条件绑定断言 |

> 说明：A-1/A-2 与上轮 A2/A1 同源（口径未完全收紧）；A-3/A-6/A-7 属「描述面/文档面」保真度与表述精度，A-4/A-5 属台账与测试强度。均不影响本轮 PASS。

---

## 4. 收据台账（机器原生）

| 项 | 命令 | 结果 |
|---|---|---|
| 复审 lint | `python3 ~/.agents/skills/dev-team/scripts/dev-review-lint.py .dev-team/review-join-acp-delegate-r2.json --scopes <14 路径>` | **[PASS] Exit 0** |
| 契约套件（本轮真跑） | `PONY_BUILD_LOCK=/tmp/pony-agent-build-acp-delegate.lock bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_delegate_p0p2_contract --test acp_cli_resume_and_epipe` | `15 passed` + `5 passed`，0 failed，**Exit 0**（`/tmp/r2-verify-tests.log`） |
| C6.2 单例复核 | 同上 `--test acp_delegate_persistence_contract -- --nocapture c6_2` | `1 passed`，**Exit 0**；无 `[C6.2] … IS NULL` 输出 ⇒ 命中 `Some` 断言分支 |
| ACP 19 套件（引用） | `/tmp/final2-acp.log` = `docs/release/evidence/acp-delegate/all-acp-final.log` | 19 × `test result: ok`、0 `FAILED`；契约套件 `:146` 15 passed；`:116` `t_acp_05`、`:134` `c0_4a2` |
| lib 全量（引用） | `/tmp/final2-lib.log` = `docs/release/evidence/acp-delegate/final-lib.log` | `:3410` `1614 passed; 0 failed`；0 个 `failures:` 段、0 个 `error[E` |
| 证据新鲜度 | `diff -q` 证据 vs `/tmp/final2-*.log`；测试二进制 hash | 两对均 **IDENTICAL**；`acp_delegate_p0p2_contract-1f31f6a165f2e780` 与本轮真跑同 hash |
| A2 现场探针 | 隔离 `HOME`/`XDG_CONFIG_HOME`：`ponycli acp --help --output-format jsonl` | Exit 0；stdout 1498B（纯 help）、stderr 0B |
| A-2 反例实测 | `python3 -c` 复刻 `find()` 子句选择逻辑 | `selected= 成功轮与截断轮都回传…`、`current_assert_passes= True`（矛盾子句逃逸） |

抽验声明：`/tmp/final2-lib.log`（1614）与 `/tmp/final2-acp.log`（19 套件）为**引用收据**；本轮实跑的是 2 个契约套件 + C6.2 单例，未重跑全量（遵守构建预算约束）。
