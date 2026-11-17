# L3-P 复审 + Join Barrier 裁决（wave `acp-delegate-p0p2`）

- 分支/工作树：`fix/acp-delegate-p0p2` @ `/home/dm/pony-agent-worktrees/acp-delegate-p0p2`
- 审查对象：union diff `c5761b27..HEAD`（62 文件），HEAD = `aaec5ae3`
- 复审触因：L3-P 前次 FAIL（F1 BLOCKING / F2·F3 ADVISORY）+ Gate-1 计划审查 FAIL（BLOCKING-1 truncated 无正文、BLOCKING-2 T-ACP-05/T-VIS-02 无冻结断言）
- 只读声明：本次审查**未改** src/测试/契约，**未做** git 写，**未跑**全量构建；仅用 `cargo-lane.sh` 跑 3 个单套件 + 只读 grep/sed + 1 次 CLI 现场探针（隔离 env，落 `/tmp`）
- 机读收据：`.dev-team/review-join-acp-delegate.json`（`dev-review-lint.py` **Exit 0**，见 §4）

## 0. 裁决

**verdict = FAIL**：1 BLOCKING / 6 ADVISORY。

根因（一句话）：B1 的行为修正（`truncated` 成功面回传 `finalText`）只同步到了 docs / ADR / 契约矩阵，**没有同步模型可见的工具描述**（`schema.rs:1145` 仍写「非 end_turn 归约（截断/拒绝/取消/错误）不返回 `finalText`，按 `error.code` 处置」）；而新增的 T-ACP-05 断言带逃逸子句，恰好对这一矛盾恒真 ⇒ 契约矩阵 C0.5「声明…与实际行为一致」被违反，且该描述正是本波要修的「模型无从消费正文契约」现场。

---

## 1. 六问基准映射否决（基准字段 vs 代码行证据 vs 判定）

| # | 六问 | 基准字段（`.dev-team/nfr-baseline-acp-delegate-p0p2.json`） | 代码行证据 / 命令 | 判定 |
|---|---|---|---|---|
| Q1 | 外部调用超时 | `external_call_timeout_ms=3000`（ACP 例外口径：单请求 5000 / 整轮预算 120s） | `acp/mod.rs:36 ACP_DEFAULT_TURN_BUDGET=120s`；本波只做「观测化」（retry/step/usage），未新增网络/外部 IO 调用点（union diff 中 ACP 侧变更全在归约与事件通道） | **PASS** |
| Q2 | 日志规范与防吞 | `logging.format=json` + `trace_id`；`acp/**` 非测试零裸 `eprintln!` | `T-LOG-01` 机器扫描保留；`otel/log.rs:148` 改道开关；F1 修复后 `bin/cli/acp.rs:52-54` 在 jsonl/sse 下改道 stderr；现场探针见 §2-F1 | **PASS** |
| Q3 | 资源释放 | `resource_release=[acp_child_process_close,pending_permissions,terminal_handles]` | `acp/session.rs` 的 `close()`/pending permission 回收语义本波未改动（union diff 中无删除）；三套件真跑绿（含 `acp_realtime_permission`/`acp_tool_permission_e2e` 在 `/tmp/final-acp.log`） | **PASS** |
| Q4 | 并发与幂等 | `concurrency_lock.type=acp_session_prompt_gate` / `bounded_wait=false` / `wait_bound=peer_turn_budget_ms` | `acp/session.rs:415 prompt_gate: Mutex<()>`、`:891` 裸 `lock()`（无 `try_lock`/无超时）；drain-on-success 幂等（提交成功才 `remove`） | **PASS**（F3 已按实现撤回 600s 声明，`dev-nfr-lint` Exit 0） |
| Q5 | 配置与自检 | 基准表 config/self-check 条目 | `acp-agents.json` 清单 + `ponycli acp --list` + `acp_config_agent_not_found` 等真源错误码（`bin/cli/acp.rs:72-79`） | **PASS** |
| Q6 | 契约兼容性 | 契约矩阵变更登记（`additive`/`breaking`） | C0.4 语义变更已登记为 additive 且 docs/ADR 同步；**但 C0.5「工具描述声明结果含 `finalText`；与实际行为一致」被违反**：`agent/tools/schema.rs:1145` vs `acp/session.rs:1432` | **FAIL**（BLOCKING，见 §2-B1） |

---

## 2. 前序遍历（F1 / F2 / F3 / B1 / B2）逐项裁决

### F1 — `ponycli acp` 日志改道 stderr：**PASS**

- 修复点：`crates/pony-agent-core/src/bin/cli/acp.rs:52-54`
  ```rust
  if parsed.output_format != OutputFormat::Text {
      pony_agent_core::agent::otel::log::set_stderr_output(true);
  }
  ```
  位置在 `parse_acp_cli_args` 之后、**所有输出路径之前**（`--list` 在 `:64`、用法错误 `acp_usage_error` 在 `:350` 全走 stderr、prompt/resume 在 `:107+`）。切换点先于 `main` 的任何结构化日志：`bin/cli.rs:2303` 先拦截 `acp`、`:2316` 才 `ObservabilityBootstrap::install` ⇒ 根因判断（子命令早退绕过改道）成立，修复位置正确。
- 幂等性：`otel/log.rs:148 set_stderr_output` 仅写 `AtomicBool`（`SeqCst`），text 模式不调用 ⇒ 逐字不变。
- 现场探针（隔离 env，fake agent `resume_no_id` 形态，触发 session/load 缺 sessionId 诊断）：
  - `--output-format jsonl`：stdout = **1 行**（`types: ['turn:delta']`），含 `"level"` 的结构化日志行 **0**；stderr = 结构化诊断 1 行（`"message":"acp session/load 响应缺 sessionId…","trace_id":null`）
  - 对照组 text 模式：stdout 携带该结构化日志行 1 行、stderr 0 ⇒ 证明确实是 F1 开关在改变分流，而非该场景本就不打日志
- R3 断言迁移：`tests/acp_cli_resume_and_epipe.rs:422-441` 已从「parse stdout」改为「parse **stderr**」，并保留「stdout 必须有 `turn:delta`」（`:421`）⇒ 若改道回退到 stdout，R3 会红。
- 套件收据（HEAD 真跑）：`acp_cli_resume_and_epipe` **5 passed/0 failed, Exit 0**；`acp_host_surface` **5 passed/0 failed, Exit 0**（含 `p2_03` 的「jsonl stdout 每行合法 JSON」，`:305`）。
- 残余（ADVISORY，见 §3-A3）：`--help` 在开关之前 `print_acp_usage()` 打 stdout。

### F2 — 终态 flush「快照→提交→移除」重复窗口：**ADVISORY-OPEN（未修复，仅字符串登记）**

- 代码未变（union diff `8afa4376..HEAD` 不含 `agent/control_plane/mod.rs`），L3-A finding[0] 的结构判断仍成立：`control_plane/mod.rs:1219` 锁内快照 → `:1225` 释放锁后提交 → `:1232` 重新加锁移除，全程无 per-key in-flight 标记。
- 登记现状：`state.json` 的 `l3p.advisories[0]` 只有一句中文描述，**无 owner / 无 backlog id / 无到期日 / 无重跑验收条款**（§3-A6）。
- 判据：NFR 未声明该窗口为基线承诺，且现存生产者拓扑下不可确定性复现 ⇒ 不构成 Q4 否决；但按 `dev-team` 的 NFR-DEGRADED 纪律，ADVISORY 欠账需带 owner+截止日才可闭环。

### F3 — NFR `concurrency_lock.timeout_ms` 与实现不符：**PASS**

- 基线已按实现改写：`nfr-baseline-acp-delegate-p0p2.json:16-21` → `bounded_wait=false`、`wait_bound=peer_turn_budget_ms`，并显式声明「本侧不实现、也不声明 600s 之类的独立锁超时」，附 `evidence` 行号。
- 实现核对：`acp/session.rs:415 prompt_gate: Mutex<()>`；`:891 let _gate = match self.prompt_gate.lock()`（无 `try_lock`、无 `timeout`、无第二套超时）；整轮预算 `acp/mod.rs:36 = 120s` ⇒ 声明与实现一致。
- 机检：`python3 ~/.agents/skills/dev-team/scripts/dev-nfr-lint.py .dev-team/nfr-baseline-acp-delegate-p0p2.json` → `[PASS]`，**Exit 0**（未知字段未被拒，schema 仍合法）。

### B1 — truncated 轮回传正文：**PASS（代码面）** / 派生 **BLOCKING（模型可见描述面）**

- 合流构造器：`acp/session.rs:1428-1439`
  ```rust
  let stop_reason_limit_hit = matches!(outcome, AcpTurnOutcome::Truncated);
  let final_text = reduce_final_text(updates);
  let usage = reduce_usage(updates);
  match outcome {
      AcpTurnOutcome::Ok | AcpTurnOutcome::Truncated => build_ok_result_with_final_text(…, stop_reason_limit_hit, …)
  ```
  ⇒ 正文/用量归约与「是否成功面」解耦；`build_ok_result_with_final_text` 恒 `payload.insert("finalText")`（`:1522`）且 `finalTextTruncated = final_text.truncated || stop_reason_limit_hit`（`:1524-1527`）。
- 失败面不产正文：`build_failure_result`（`:1541`）刻意不含 `finalText` 键；`c0_4b/c0_4c/c0_4d` 用 `assert_no_final_text_contract`（`:408-422`）机器断言键缺席。
- 断言实测：`c0_4a_truncated_stop_reason_carries_final_text`（`tests/acp_delegate_p0p2_contract.rs:425`）断言 `truncated=true`、`stopReason=max_tokens`、`finalText` 存在（空串）、`finalTextTruncated=true`、summary 诚实提示；套件 **14 passed/0 failed, Exit 0**。
- **BLOCKING（本次新增，唯一否决项）**：`crates/pony-agent-core/src/agent/tools/schema.rs:1145` 的模型可见 `description` 仍宣称
  「非 end_turn 归约（**截断**/拒绝/取消/错误）**不返回** `finalText`，按 `error.code` 处置」
  与以下三处真源矛盾：
  - 实现：`acp/session.rs:1432`（`Ok | Truncated` 同一构造器）；
  - 文档：`docs/architecture/acp-client-integration.md:168`「`truncated`（撞输出/轮次上限）仍回传已产出的 `finalText` 并置 `finalTextTruncated=true`；**仅** refused/cancelled/error 不产 `finalText`」；
  - 契约：`contract-matrix-acp-delegate-p0p2.md` C0.5「工具描述（模型可见）声明结果含 `finalText` 正文；**与实际行为一致**」。
  影响：`max_tokens/max_turn_requests` 轮是 `ok` 成功面且**无** `error.code`，模型按该描述会把正文丢弃并把成功轮当错误处置 —— 正是本波 C0.5 立项要消灭的「模型只能靠猜」。
  反例命令：见 JSON `findings[0]`（replay Exit 0：同时打印描述原文、`session.rs:1428-1438` 合流与 docs:168）。
  修复面（一句话）：同步 `schema.rs:1145` 描述与 `:1142-1144` 注释，改为「`truncated` 仍回传 `finalText` 且 `finalTextTruncated=true`；仅 refused/cancelled/error 不产正文并按 `error.code` 处置」。

### B2 — Gate-1 冻结断言：**PARTIAL**

- T-ACP-05：`tests/acp_delegate_p0p2_contract.rs:1412 t_acp_05_model_visible_description_declares_final_text_contract` —— **真实新增、非空断言、非 skip**（无 `#[ignore]`，文件头 `:30` 明文禁 `#[ignore]`/`assert!(true)`）。断言取 `surface_tool_definitions()` 的模型可见面，逐个 token 断言 `finalText` / `finalTextTruncated` / 「正文」。
- **但其逃逸子句使断言对本次矛盾恒真**（`:1439-1442`）：
  ```rust
  assert!(!description.contains("不返回 `finalText`") || description.contains("非 end_turn"), …)
  ```
  实测描述**同时**含「不返回 `finalText`」与「非 end_turn」⇒ 右支为真 ⇒ PASS，矛盾无法被拦。这正是 §2-B1 BLOCKING 漏网的原因（不是「没加断言」，而是「断言的口径放过了要冻的东西」）。
- T-VIS-02：`:1513 t_vis_02_external_chunk_text_never_enters_parent_assistant_stream` —— 真实有效：写唯一标记 chunk + tool_call，注册 `register_event_persist` 捕获父事件流，**先做正控**（工具结果 `finalText` 必须含标记，`:1555-1559`），再断言父 `AssistantChunk/AssistantMessage` 零泄漏（`:1562-1577`）；用 `EVENT_PERSIST_LOCK` 串行化全局单槽，无 sleep 盲等。**非空、非假绿**。

---

## 3. 残余 ADVISORY 清单（6 条，均带文件:行 + 反例命令，见 JSON）

| id | 位置 | 内容 | 处置建议 |
|---|---|---|---|
| A1 | `tests/acp_delegate_p0p2_contract.rs:1439` | T-ACP-05 逃逸子句 `!contains("不返回 finalText") \|\| contains("非 end_turn")` 对「非 end_turn 里包含 truncated」这一真实矛盾恒真 ⇒ 冻结断言对 B1 语义变更失效 | 改为显式断言：描述不得把 `truncated` 列入「不返回 `finalText`」集合；或直接断言描述含「`truncated`…仍回传」 |
| A2 | `bin/cli/acp.rs:166`（+`:545 print_acp_usage`） | `--help` 在改道开关（`:52`）**之前**把 usage 打到 stdout；实测 `ponycli acp --output-format jsonl --help` → stdout 1498B 文本、stderr 0B，与帮助文本自述「jsonl/sse: stdout = events only」不符 | 二选一：help 也走 stderr（除 text 模式），或把「stdout 纯事件」契约显式豁免 `--help` 并在契约矩阵登记 |
| A3 | `tests/acp_cli_resume_and_epipe.rs:422` | 注释把改道点写成 `src/bin/cli.rs ObservabilityBootstrap::install`，但 `acp` 子命令在 `main:2303` 早退、**从不**经过 bootstrap，真实调用点是 `bin/cli/acp.rs:52` ⇒ 误导维护者；且 R3 只断言「stderr 有诊断」，**未**断言「stdout 无诊断」（双流泄漏不会被抓） | 更正注释；补一条 stdout 负向断言（stdout 无 `"level"` 键行） |
| A4 | `tests/acp_delegate_p0p2_contract.rs:425` | `c0_4a` 的 fake agent **不发** `AgentMessageChunk`，`finalText` 断言的是空串 ⇒ 「truncated 轮带着非空正文成功回传」这条主路径无覆盖（代码为共享归约路径，风险低但断言面窄） | 追加「truncated + 非空 chunk」剧本，断言 `finalText == 正文` 且 `finalTextTruncated=true` |
| A5 | `.dev-team/state.json:179`（`l3p.advisories`） | 仅 2 条裸字符串：F2 无 owner/backlog/到期日；L3-P 第 3 条（`tests/acp_delegate_persistence_contract.rs:628`：`created_at_ms IS NULL` 时只 `println!` 空过，C6.2 时间语义失守）**完全未登记**；L3-A/L3-B 的 12 条 ADVISORY 也无逐条台账（meta-retro `:9` 声称「已登记」，实际无条目） | 建 `.dev-team/advisories-acp-delegate.json`（id/owner/截止/重跑验收），F2、L3-P#3、L3-A/B 12 条逐条入册 |
| A6 | `docs/release/evidence/acp-delegate/all-acp-final.log:126` | 归档证据是**修复前**运行：出现旧用例名 `c0_4a_truncated_stop_reason_unchanged`、缺 `t_acp_05_*`；修复后的绿收据只在 `/tmp/final-acp.log`（进程类产物）⇒ Phase-4「证据类入仓」尚未刷新 | 用 `/tmp/final-acp.log` 覆盖该证据文件并同提交引用（本次只读，未代做） |

> 说明：A5/A6 属「登记与归档」缺口，不影响代码正确性；A1 与 §2-B1 的 BLOCKING 同源，修 BLOCKING 时必须一并修 A1，否则同类矛盾会再次漏网。

---

## 4. 收据台账（机器原生）

| 项 | 命令 | 结果 |
|---|---|---|
| 复审 lint | `python3 ~/.agents/skills/dev-team/scripts/dev-review-lint.py .dev-team/review-join-acp-delegate.json --scopes <62 文件>` | **[PASS] Exit 0** |
| 契约套件 | `PONY_BUILD_LOCK=… bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_delegate_p0p2_contract` | `14 passed; 0 failed`，**Exit 0** |
| CLI resume/EPIPE | 同上 `--test acp_cli_resume_and_epipe` | `5 passed; 0 failed`，**Exit 0** |
| ACP 宿主面 | 同上 `--test acp_host_surface` | `5 passed; 0 failed`，**Exit 0** |
| lib 全量（引用） | `/tmp/final-lib.log` | `test result: ok. 1614 passed; 0 failed`（唯一一条，`:3410`） |
| ACP 19 套件（引用） | `/tmp/final-acp.log` | 19 个 `test result: ok`、0 个 FAILED；且**含新用例** `t_acp_05`（`:116`）、`c0_4a_…_carries_final_text`（`:127`）⇒ 确为修复后运行 |
| NFR schema | `python3 …/dev-nfr-lint.py .dev-team/nfr-baseline-acp-delegate-p0p2.json` | `[PASS]`，**Exit 0** |
| F1 现场探针 | 隔离 HOME/XDG + `acp_fake_agent.sh resume_no_id`，`--output-format jsonl` vs text | jsonl：stdout 仅 `turn:delta`、诊断在 stderr；text：诊断在 stdout（对照组） |

抽验声明：`/tmp/final-lib.log` 与 `/tmp/final-acp.log` 的「1614/19 套件」为引用收据，本次抽验了「新用例确实出现在 final-acp.log」这一条（`grep t_acp_05 / c0_4a`），并另真跑 3 个单套件复核，未重跑全量（遵守禁全量构建约束）。
