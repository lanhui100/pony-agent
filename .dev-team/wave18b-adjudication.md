# wave-18b 审查 findings 逐条裁定表（T4 / L2-P 协议特派员）

- 任务：`task-4`（wave-18b 契约升版 r3）
- 契约基线：`.dev-team/contract-matrix-wave18.md` revision **`wave18-r3`**
- 证据源（4 路 L3/L3-P 审查，**全部 verdict = FAIL**）：
  1. `.dev-team/review-l3-contract-wave18.json`（4 findings）
  2. `.dev-team/review-l3-scanner-wave18.json`（5 findings）
  3. `.dev-team/review-l3-security-wave18.json`（1 finding）
  4. `.dev-team/review-l3p-wave18.json`（2 findings）
  - **合计 12 项 findings**（其中 contract#3 与 security#1 为**同一缺陷的两路独立复现**）。
- 裁定口径（Lead）：**FIX 8 项 · WAIVE 1 项 · 既有缺陷纳入 1 项 · 登记 3 项**。
- 责任任务（共享任务板实况）：`task-5` = T5 test（红相，`blocked_by: task-4`）· `task-6` = T6 executor（实施，`blocked_by: task-5`）· `task-2`/`task-3` 已完成。

---

## 1. 裁定总表（12/12 覆盖）

| # | 来源 finding | 位置 | 缺陷摘要 | 裁定 | 责任任务 | 契约落点 / 证据 |
|---|---|---|---|---|---|---|
| 1 | `review-l3-contract` #1 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:137` | `test_upload_session_trace_contract_shape` 函数体仅 2 行注释、零断言，`cargo test` 报 `ok`（零假测试） | **FIX** | **task-5**（补真断言） | r3 §11 M12；证据：review 文件 `counterexample` 字段（`git show 98bd584` / `d3ae148` 逐字相同） |
| 2 | `review-l3-contract` #2 | 同上 `:151-181`（`[skip]` 打印于 `:159`/`:169`/`:177`） | 三个 early `return` 使 `:224-238` 的 201/401/404 断言永不执行，却以 `test ... ok` 上报（等价 `#[ignore]` 但报 `ok`） | **FIX** | **task-5**（拆"恒跑真断言 + env 门控 live"两段） | r3 §9.2 R6 + §11 M13；证据：review `counterexample`（`env -u ... cargo test ... => [skip] ... ok`） |
| 3 | `review-l3-contract` #3 **+** `review-l3-security` #1（两路独立复现同一缺陷） | `management/task-system/00_DASHBOARD.md:23` | 断言"全仓 `grep -rn "MAX_SUBAGENT_DEPTH"` 零命中"为假（实测 2 命中，1 处即该断言行自身，1 处为 `03_TASKS/PA-110-...md:26`） | **FIX（既有缺陷纳入本波）** | **Lead 指派**（原 `task-3` 写域，当前无 pending 任务持有该路径） | r3 §9.2 R7；证据：review-l3-security `counterexample`（`grep -rn ... → 2 命中`，exit=0） |
| 4 | `review-l3-contract` #4 | `crates/pony-agent-core/src/agent/ponysentry/config.rs:41-43` | `PONYSENTRY_ENABLED=""`/纯空白 => `true`（fail-open）；`.unwrap_or(false)` 只兜住 unset；与 C5 前端"空串按未设"不对称 | **FIX** | **task-5**（红相）+ **task-6**（实施） | r3 §2（断言 7/8/9 + 冻结实现形态）；证据：review `counterexample`（`L3PROBE blank PONYSENTRY_ENABLED => enabled=true`，Exit 101） |
| 5 | `review-l3-scanner` #1 | `scripts/check-secrets.mjs:102` | 豁免 3（Markdown 单元格）整行放行且无 `credWord` 守卫 → `\| PONYSENTRY_CLIENT_TOKEN \| <64hex> \|` = 0 命中 | **FIX** | **task-5**（红相）+ **task-6**（实施） | r3 §4.2 C4-R1 + §4.7 G1/G2；独立复现见 §3 |
| 6 | `review-l3-scanner` #2 | 同上 `:98`（豁免 1）与 `:100`（豁免 2） | 豁免 1/2 整行放行且无守卫 → `{"client_token":"<64hex>","integrity":...}` = 0；`<hex>  PONYSENTRY_CLIENT_TOKEN=<hex>` = 0（第二个具名 token 被吞） | **FIX** | **task-5**（红相）+ **task-6**（实施） | r3 §4.2 C4-R1/R2 + §4.7 G3–G6；独立复现见 §3 |
| 7 | `review-l3-scanner` #3 | 同上 `:162`（调用点）+ `:77-83`（`Private Key Block` 规则，`isSafe` `:79-82`） | `isSafe(v, line) \|\| isSafe(v, content)` 使私钥规则在**整文件**上下文判定 `TEST\|MOCK\|DUMMY\|EXAMPLE` → 真实私钥只要同文件出现 test 字样即 100% 漏报（`--staged`/`--all` 均 Exit 0） | **FIX（既有缺陷纳入本波）** | **task-5**（红相）+ **task-6**（实施） | r3 **§7 C7**（含反熵理由 §7.2 / §12.3）；独立复现见 §3 |
| 8 | `review-l3-scanner` #4 | 同上 `:96`（`line.toLowerCase()`）、`:98`（`regex.test`）、`:105`（`line.includes`） | 每 match 重复整行运算 → `O(matches × 行长)` 二次方；1MB 单行 = **133,411.9 ms** / 15000 违规 | **FIX** | **task-5**（红相）+ **task-6**（实施） | r3 §4.2 C4-R3 + **§4.6 性能硬约束**（≤100ms / 4000×64hex）；独立复现见 §3 |
| 9 | `review-l3-scanner` #5 | `package.json:39`（`secrets:check` = `--staged`）、`:41`（`verify` 串联） | `npm run verify` 在含**未暂存**泄漏的工作区上给假绿（`--staged` 输入为 0） | **WAIVE（登记欠账）** | — （登记 `DEBT-W18-R4`） | r3 §9.2 R4；证据：review `counterexample` + 已核实的缓解事实（`ci.yml:44` `--all`；pre-commit/pre-push hooks） |
| 10 | `review-l3p` #1 | `src/lib/telemetry.ts` ingest `fetch`（r3 前 `:289`） | 前端 fetch 无 `AbortSignal`/硬超时（`grep -nE "AbortController\|AbortSignal\|setTimeout\|signal"` 零命中，EXIT=1）→ 无界外部调用，突破 `external_call_timeout_ms ≤ 3000` | **FIX** | **task-5**（红相）+ **task-6**（实施） | r3 **§8 C8**；达标对照 `client.rs:14` `REQUEST_TIMEOUT_SECS = 3` |
| 11 | `review-l3p` #2 | `.env.example`（不存在，`line: 0`） | 仓库无配置模板；`PONYSENTRY_ENABLED` 默认翻转 false 后，部署方在仓库内无任何线索得知须显式置 true | **FIX** | **task-5**（红相）+ **task-6**（实施） | r3 **§6 C6**；证据：review 命令1（`ls .env.example` EXIT=2）/ 命令2（`git ls-files \| grep '^[.]env'` 零命中 EXIT=1） |
| 12 | `review-l3-contract` #3（同 #3） | 同 #3 | 见 #3（同一缺陷，security 路复现） | **计入 #3（既有缺陷纳入本波）** | 同 #3 | 见 #3 |

> **12 项 findings 全部有裁定，无遗漏。**

---

## 2. FIX 项明细（8 项裁定 → 覆盖 9 项 findings）

| FIX | 覆盖 finding | 契约落点 | 实施责任 | 红相责任 | 验收判据 |
|---|---|---|---|---|---|
| F1 C2 fail-closed 边界 | #4 | §2 | task-6 | task-5 | `""`/`"   "`/`"\t"` => `false`；`" false "`/`"  0  "` => `false`；`"true"`/`"1"` => `true`；未设 => `false`。**变异敏感**：退回 `.map(...).unwrap_or(false)` 则 set-but-empty 断言必失败 |
| F2 C4 豁免守卫（credWord） | #5 + #6（合并为一项） | §4.2 R1 / §4.3 | task-6 | task-5 | §4.7 G1–G6 六条必须 ≥1 命中；且无凭证语义词的纯摘要上下文仍 0 命中（零误报不回退） |
| F3 C4 豁免位置锚定 | #6 | §4.2 R2 / §4.3 | task-6 | task-5 | G6（一行两个 hex）中第二个具名 token 必须命中 |
| F4 C4 线性化 | #8 | §4.6 | task-6 | task-5 | 4000×64hex 单行 ≤100ms（基线 2072.4ms）；1MB 单行 15000 hex 显著下降；命中数正确；缓存有界 ≤4 |
| F5 C7 私钥块级豁免 | #7 | §7 | task-6 | task-5 | 真实块 + 远端 `test` 字样 => 命中；`// MOCK TEST KEY` 紧邻上一行 => 0 命中（`spec:83-92` 不破）；多块文件无首块歧义 |
| F6 C8 前端硬超时 | #10 | §8 | task-6 | task-5 | `signal` 已挂载 + `setTimeout`/`clearTimeout` 存在 + 常量 ≤3000；删 `signal`/`setTimeout` 断言必失败 |
| F7 C6 配置模板 | #11 | §6 | task-6 | task-5 | `.env.example` 被跟踪、含 5 变量名与自检命令、零 64-hex、`--all` Exit 0 |
| F8 R5/R6 假测试与静默 skip | #1 + #2 | §11 M12/M13 | task-5（测试写域） | task-5 | M12 补真断言（注释掉断言则测试失败）；M13 拆两段，恒跑段有真断言、门控段如实上报 `ignored` 而非 `ok` |

---

## 3. WAIVE / 登记 / 既有缺陷

### 3.1 WAIVE 1 项

| 项 | finding | 裁定 | 欠账编号 | 到期条件 | 证据 |
|---|---|---|---|---|---|
| W1 | `review-l3-scanner` #5（`npm run verify` 的 `--staged` 假绿） | **WAIVE（本波不修）** —— 属项目级门禁语义变更，且 CI（`.github/workflows/ci.yml:44` `--all`）与 pre-commit/pre-push hooks 已分别覆盖，超出本波"凭证清理 + 扫描器补盲"最小闭包 | **`DEBT-W18-R4`** | 下一次涉及门禁/CI 语义的波次 | `review-l3-scanner-wave18.json` finding#5 |

### 3.2 既有缺陷纳入本波 1 项

| 项 | finding | 裁定 | 说明 |
|---|---|---|---|
| P1 | contract#3 / security#1（`00_DASHBOARD.md:23` 断言为假） | **FIX（纳入本波）** | 该文件不在 `.dev-team/` 写域，本特派员**不修改**；由 Lead 指派执行方。修正后必须自洽：改后重跑同一 `grep -rn "MAX_SUBAGENT_DEPTH"`，命中数须与文本陈述一致（正确表述："全仓**代码**（`*.rs`/`*.ts`）零命中"） |

### 3.3 登记 3 项（非 findings，r2 保留）

| 项 | 内容 | 状态 | 登记位置 |
|---|---|---|---|
| R1 | 32-hex **不作"必须命中"**要求；C4 维持严格 64 位窗口 | **backlog**（待独立评估长度窗口与豁免口径） | r3 §9.2 R1 |
| R2 | `tests/acceptance/ponysentry_telemetry_test.rs` 为**孤儿死文件**（不被 cargo 编译、不被 vitest 收集） | **dead-file backlog**（删除 或 纳入真实编译目标，待 Lead 决策）；断言仍须迁移 | r3 §9.2 R2 + §11 M4/M5/M11 |
| R3 | 泄漏面复核：跟踪文件恰好 3 文件 4 处，`.dev-team/` 内不含该 token | **已复核，一致** | r3 §9.2 R3 |

---

## 4. 计数核对（与 Lead 裁定口径一致）

| 口径 | 数量 | 对应 finding |
|---|---|---|
| FIX | **8** | #1, #2, #4, #5+#6（合并为 1 项修复）, #7, #8, #10, #11 → 覆盖 **9** 项 findings |
| WAIVE | **1** | #9 |
| 既有缺陷纳入本波 | **1** | #3（= contract#3 + security#1，两路复现**同一缺陷**，覆盖 **2** 项 findings） |
| 登记（非 findings） | **3** | R1 / R2 / R3 |
| **findings 总数** | **12** | 9 + 1 + 2 = **12** ✅ |

---

## 5. 独立复现收据（本特派员亲自执行，非引用）

探针 `/tmp/w18b-probe.mjs`（只读，未写入仓库；64-hex 由分片拼接构造，探针文件本身不含完整字面量）。

| 命令 | 真实输出摘要 | Exit | 结论 |
|---|---|---|---|
| `node /tmp/w18b-probe.mjs` | G1/G2/G3/G4/G5/G6 全部 `hits=0`（期望 ≥1）→ `FAIL`；控制组 bare/quoted 64-hex `hits=1` → PASS；git-sha-40 / git-sha-7 / sha256 关键词行 `hits=0` → PASS | **1** | findings #5/#6 复现确认 |
| 同上（`finding#3` 行） | `private-key alone hits=1 (want 1) / with 'test' word hits=0 (want 0 => blind)` → PASS | — | finding #7 复现确认（失明） |
| 同上（`finding#4` 行） | 单行 4000×64hex（259,999 B）= **2072.4 ms**；拆行 = **19.7 ms**；`ratio_single_over_multi=105.0x` | — | finding #8 复现确认（二次方） |
| `pk.isSafe(BEGIN, BEGIN)` / `isSafe(BEGIN, "// MOCK TEST KEY\n"+BEGIN)` / `isSafe(BEGIN, <真实块 + 远端 test>)` | `false` / `true` / `true` | **0** | §7.2 论证：字面"1 行删除"会破坏 `spec:83-92`；有界窗口可同时满足"报真私钥 + 不破 mock 豁免" |
| `grep -rn "acceptance" Cargo.toml crates/*/Cargo.toml src-tauri/Cargo.toml` | 零命中 | **1** | R2 双路证据之一 |
| `grep -n "include\|exclude" vitest.config.ts` | `:20 include: ["tests/**/*.spec.ts", "tests/**/*.test.ts"]`；`:21 exclude: ["tests/e2e/**"]` | 0 | R2 双路证据之二 |
| `grep -nE "AbortController\|AbortSignal\|setTimeout\|signal" src/lib/telemetry.ts` | 零命中 | **1** | finding #10 复现确认 |
| `ls .env.example` | `No such file or directory` | **2** | finding #11 复现确认 |
| `python3 .dev-team/nfr-lint.py --instance .dev-team/nfr-baseline-wave18.json` | `RESULT: PASS` | **0** | r3 基线验收 |
| 反例注入（`timeout_ms=5000` + `per_match_whole_line_work_forbidden=false` + `set_but_empty_is_false=false`） | `RESULT: FAIL (3 error(s))` | **1** | 新基线字段非恒真 |

---

## 6. 裁定后的执行顺序（依赖约束）

```
task-4（本文件，r3 契约冻结）  ✅
   └─ task-5（红相：8 项 FIX 的失败测试 + 变异敏感收据）
         └─ task-6（实施：C2/C4/C6/C7/C8）
               └─ 复验（L3/L3-P 复审 + Lead 终审）
并行：P1（看板修正）由 Lead 指派；W1/R1/R2/R3 为欠账登记，不阻塞本波
```
