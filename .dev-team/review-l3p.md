# L3-P 生产就绪审查（六问基准映射否决制）— wave-acp-delegate-p0p2

- 工作区：`/home/dm/pony-agent-worktrees/acp-delegate-p0p2`（分支 `fix/acp-delegate-p0p2`）
- 基线：`c5761b27`；审查执行时 HEAD＝`8afa4376`（docs 提交），代码面与之上一提交 `fbde9fe7` 逐字一致
  （`git diff --name-only fbde9fe7..8afa4376` 仅 docs/ADR/evidence/state.json）
- 范围口径差异：任务书引用「51 文件 / +5042」（对应 `fbde9fe7`）；实际 union diff `c5761b27..8afa4376`
  ＝**55 文件 / +8817 / −284**，多出的 4 个文件是 `docs/architecture/acp-client-integration.md`、
  `.agents/notes/implemented/bug-fix/2026-11-17-…md`（proposed→implemented 改名）、
  `docs/release/evidence/acp-delegate/{all-acp-final,full-lib-w2w3,c6-green}.log`。本审查按 55 文件闭合。
- 只读：未改 src/测试、未做 git 写、未做构建；仅写 `.dev-team/review-l3p.{md,json}`。
- 客观证据（仅引用白名单实体）：
  - `.dev-team/red-phase/{acp_delegate_p0p2_contract,acp_delegate_adversarial,acp_delegate_persistence_contract}.receipt.json`（红相 Exit 101，真实断言失败）
  - `/tmp/full-lib-w2w3.log`＝`test result: ok. 1614 passed; 0 failed`
  - `/tmp/all-acp-final.log`＝19 个 ACP 套件全绿（19×`test result: ok`，0×FAILED，含 3 个新套件）
  - `.dev-team/regression-baseline-acp-delegate.json`（`--lib` 基线 1607 passed 快照）
  - 仓库内 `docs/release/evidence/acp-delegate/*.log` 与 `/tmp` 同名日志 **md5 逐一相同**（取证未被篡改）

## 一、六问对照表

| # | 基准字段（真源） | 代码行证据 | 判定 |
|---|---|---|---|
| 1 | `external_call_timeout_ms=3000`（全局 acp 段：`request_timeout_ms=5000` / `handshake_timeout_ms=30000` / `spawn_reap_grace_ms=500`） | 本次**未新增任何外部网络往返**：变更面是 ACP 诊断从 `eprintln!` 迁到 `agent::otel::log`（本地写）+ 结果归约纯函数。既有硬超时常量与全局基线逐值一致：`acp/transport.rs:38` =5000、`:45` =30000、`:75` =500；整轮预算 `acp/mod.rs:36` =120s。5000ms 的例外口径已**显式登记**：`nfr-baseline-acp-delegate-p0p2.json:9-10`（`external_call_timeout_scope` 声明"沿用全局基线的 acp 段，不构成对本字段的重新解释"）↔ `nfr-baseline.json` 的 `acp.request_timeout_ms=5000`。日志 stdout 无 deadline 属**预存豁免**：`.dev-team/contract-matrix-wave59-headless.md:59`（P-BLOCKING→ADVISORY，"stdout 背压不在有界执行承诺内"）。 | **PASS** |
| 2 | `logging.format=json` + `trace_id`（C3.1） | `grep -rn 'eprintln!\|println!' crates/pony-agent-core/src/acp/` ⇒ **0 命中**；迁移后 16 处调用全部走 `agent::otel::log::{log_json,log_json_with_fields}`（client.rs:281/607/624/700/725、session.rs:39+18 处、config.rs:432/473/595/624/642、handlers.rs:118/541、transport.rs:691）。CLI `--resume` 补一条结构化诊断 `bin/cli.rs:1372-1381`。fail-closed 路径仍是"记日志 + 拒绝"而非吞异常（session.rs:260-268 审批往返失败、session.rs:334-342 CAS 消失、session.rs:1599-1607 置 Cancelled 失败）。`trace_id` 键恒存在（`agent/otel/log.rs:build_json_line`，未显式传则回落 `current_trace_id`）；实测 `/tmp/all-acp-final.log` 57 行含 `trace_id`，其中 10 行是真实 32 位 trace（含新增 `turn events flushed at step boundary`），47 行为无活跃 turn 的 helper 子进程（语义允许的 null）。 | **PASS** |
| 3 | `resource_release = [acp_child_process_close, pending_permissions, terminal_handles]` | 委派收尾仍无条件 `session.close()`（`agent/tools/orchestration.rs:249-251`），`timeoutMs=0` 早退路径也先 close（`:208`）；本次 diff **未触碰**子进程 close/pending permission/terminal 释放实现（diff 内无相关删除，`acp/session.rs:1022/1056/1063` 清理逻辑原样）。新增状态 `parent_turn: Mutex<Option<ParentTurnRef>>` 挂**每次委派新建**的 session（`orchestration.rs:185-197` 每任务 `spawn_with_full_options`，无复用池），随 session 一起释放，不跨轮残留。增量 flush 的缓冲释放是 commit-先于-clear：`control_plane/mod.rs:1219`（快照）→`1232`（成功才 `buf.remove`），失败保留可重试（不丢事件、不双写新键）。 | **PASS**（并发重复窗口单列 F2） |
| 4 | `concurrency_lock = acp_session_prompt_gate / timeout_ms=600000` | ①串行闸存在但**无等待上限**：`acp/session.rs:891` 是阻塞式 `self.prompt_gate.lock()`（仅处理中毒），与基准 `nfr-baseline-acp-delegate-p0p2.json:18-19`「锁等待上限 600000ms」不符 ⇒ **F3**。②事件 seq：`emit_external_turn_event` 与 `emit_event` 共用 turn 级加锁计数器（`turn_flow.rs:861-867`），不会撞号；落库 seq 由 `store_metadata` 计数器在事务内分配（`sqlite_session.rs:1513-1524` 缺失时以 `MAX(seq)+1` 修复），两条链各自单调唯一。③drain-on-success：单轮顺序路径无重复（冻结用例 `tests/acp_delegate_persistence_contract.rs:556-568` 显式断言"同一事件不得重复落盘"），但**会话级** drain（`control_plane/mod.rs:1254` 按 session 取全部 key）与终态快照（`:1219`）之间仍有既有"快照→提交→移除"窗口，被 step 边界抬高触发频次 ⇒ **F2**（ADR 残余风险①已自认，留待本审复核）。 | **FAIL**（F3 基准字段无实现；F2 并发窗口） |
| 5 | 基准表/自检：是否引入新配置键、缺省是否安全 | 本次**零新增用户配置键**：diff 内无新 `env::var` / settings 字段 / 配置解析分支（`acp/config.rs` 全是日志迁移）。新增常量只一条 `ACP_DELEGATE_FINAL_TEXT_MAX_CHARS=32_000`（`acp/result.rs:25`，单一真源，契约 C0.3 已登记）。`eventTimes` 缺省链安全：空数组 ⇒ 键**缺席**（`session/log_command.rs:238-247`）⇒ 旧读方逐字不变；旧入口 `store.persist_events` 传空（`store.rs:2683-2695`）；落库逐事件取值、缺失/0 回落批首时间（`sqlite_session.rs:1519-1524`）；重放读取端只取 `events`（`session/fold.rs:573-576`），未知键忽略。 | **PASS** |
| 6 | additive 登记：finalText / usage / parent_activity_id / eventTimes / ProviderCallCacheRecord 新字段 / ToolCallStatus::Cancelled | ①`finalText`+`finalTextTruncated`+`usage` 只在 `end_turn` 分支追加（`acp/session.rs:1425-1435` Ok→新构造；`:1480-1540` 键集＝旧 9 键逐一保留 + 新键；`usage` 无 `usage_update` 时**键缺席**不伪造）；非 end_turn 归约逐字不变（`Truncated/Refused/Cancelled/Error` 仍走 `build_ok_result`/`build_failure_result`）。②`parent_activity_id`：`Option`+`skip_serializing_if`（`turn_event.rs:153-159/177-181`），`TurnEvent` 无 `deny_unknown_fields` ⇒ 旧日志可读、新日志旧读方忽略。③`ProviderCallCacheRecord.attempts/failed_attempt_ms/prelude_ms`：`#[serde(default, skip_serializing_if="Option::is_none")]`（`telemetry.rs:138-155`），口径单点 `ProviderCallRetryMeta`（`:157-197`）。④`ToolCallStatus::Cancelled`（`protocol.rs:520-527`，snake_case）把"整帧反序列化失败"变为可解析，属增强。⑤`eventTimes` additive 且空即缺席。⑥**无字段删除、无变体重编号、无 wire 枚举改号**（diff 内 TurnEvent 变体顺序未动）。**但**：C3.1 的"诊断介质 stderr→stdout"未在既有契约登记——`contract-matrix-acp-cli-resume.md:12` 仍写"响应缺 sessionId ⇒ 沿用请求 id + **stderr 诊断**"，冻结断言被改写为 stdout（`tests/acp_cli_resume_and_epipe.rs:422-440`、`tests/acp_session_lifecycle.rs:225-258`），且 `ponycli acp` 路径在 `set_stderr_output` 之前就 return（`bin/cli.rs:2303` < `:2316`）⇒ 结构化模式下 stdout 混入诊断行 ⇒ **F1（BLOCKING）**。 | **FAIL**（F1） |

## 二、结论

**verdict = FAIL**：1 条 BLOCKING（F1，契约兼容/输出介质）+ 3 条 ADVISORY（F2 并发重复窗口、F3 基准字段无实现、F4 验收软断言）。

六问中 1/2/3/5 通过，4/6 不通过。四条已登记契约（C0.1–C6.3）本身均已落地并被冻结用例覆盖（红相 101→绿相全绿），BLOCKING 不在功能面而在**输出介质契约**：一次本可三行修好的路由缺失，把 `jsonl/sse` 的 stdout 纯净承诺（wave59 C1）打破了，并用"改写断言"而不是"登记契约变更"收口。

### F1（BLOCKING，CONTRACT_BREAK）`ponycli acp --output-format jsonl` stdout 混入诊断行

- 位置：`crates/pony-agent-core/src/bin/cli.rs:2303`（`run_acp_subcommand_if_requested()` 在 `:2316` `ObservabilityBootstrap::install()` 之前 return）
- 机理：`acp` 路径从不调用 `log::set_stderr_output(true)`（`bin/cli/acp.rs` 无该调用），而 `agent/otel/log.rs:157` 的缺省 sink 是 **stdout**；本次把 32 处 ACP 诊断从 `eprintln!`(stderr) 迁到 `otel::log`，于是诊断进入 `--output-format jsonl` 的事件流。
- 冲突契约：`.dev-team/contract-matrix-wave59-headless.md:18`（C1）「`jsonl`/`sse` 下 **stdout 只承载事件**，横幅/诊断一律转 stderr」；`.dev-team/contract-matrix-acp-cli-resume.md:12`「stderr 诊断」。
- 现状取证：`tests/acp_cli_resume_and_epipe.rs:422-425` 自述「本 `acp` 子命令路径未触发 `set_stderr_output` 改道 ⇒ 实测诊断在 stdout，与 turn 事件 JSONL 同流」。
- 建议修法（≤3 行，不改语义）：`run_acp_client_mode` 解析出 `output_format != Text` 时先 `log::set_stderr_output(true)`；并把 `contract-matrix-acp-cli-resume.md:12` 的"stderr 诊断"同步改写为"结构化 JSON 诊断（stderr）"。

### F2（ADVISORY，CONCURRENCY）会话级 drain 与终态快照的重复写窗口被增量 flush 抬高频次

`control_plane/mod.rs:1219` 终态先克隆快照、`:1229` 释放缓冲锁后提交、`:1232` 成功再移除；而 `:1254` 的 `flush_requester` 按 **session** 取走该会话所有 key（含另一轮在飞的 (session,turn) 半成品）。当同会话两轮并发时，"A 快照→B step-flush 提交同一批→A 再提交快照"会写出同一批事件的第二份行（新 seq）。属既有结构（历史/ask-resume 路径已可触发），本次把触发点从"轮末一次"抬到"每 hop 一次"，ADR 残余风险①已声明但未登记为可验收项。因同会话并发轮次在生产路径无正向入口（ACP 侧有 600s 串行闸，session 每次委派新建），按 ADVISORY 记；建议后续在 `commit_event_batch` 前做"缓冲世代号/generation"校验或以 turn 级 key 精确 drain。

### F3（ADVISORY，NFR_VIOLATION）基准声明的闸门等待上限无实现

`nfr-baseline-acp-delegate-p0p2.json:18-19` 声明 `concurrency_lock.timeout_ms=600000`；`acp/session.rs:891` 是裸 `Mutex::lock()`（无 `try_lock_for`/无超时）。实际可用性是"由单轮预算 120s + ≤100ms 取消轮询间接兜住"，不会真等 600s，但**基准字段与代码行不一致**，属"承诺未配机械可查实现"。建议二选一：改基准为"由 turn 预算间接有界、无独立闸门超时"，或实现带超时的闸门。

### F4（ADVISORY，CORRECTNESS）C6.2 会话 `createdAt` 半条断言是软断言

`tests/acp_delegate_persistence_contract.rs:620-629`：列非空才断言 `created_at <= 首条事件`，为 NULL 时只 `println!`。契约 C6.2 的"会话 createdAt = 会话创建时刻"因此可能静默不被验证（用例已诚实登记该限制）。建议补一条必然走到的断言（例如 fixture 先建会话行，或断言 flush 后该列必为 Some）。

## 三、非 finding 的观察（供 Lead 记录）

1. `eventTimes` 目前是**只写不读**的落盘键（写侧 `log_command.rs:241`，读侧 `fold.rs:573` 只取 `events`）。它的价值是"未来读方/取证"与文件后端 header 的 `createdAt` 提示（`log_sink.rs:200-230`），不构成兼容负担；但"旧日志时间不可回溯"（ADR 残余风险③）在重建路径无补救。
2. step 边界 flush 发生在**下一 hop 的 `turn:trace`(StepStart) 已入缓冲之后**（`runtime/turn_stream.rs:815-852`），注释写"上一 hop 已闭合"，实际多带一条下一 hop 的起点事件。语义无害（C6.1 只要求"截至该 step 的事件可读"），但注释与行为不完全对齐。
3. `emit_external_turn_event` 先取号（`turn_flow.rs:1514-1522`）再判 `build_turn_event` 是否 None；不落盘的类型会白吃一个 turn 序号（序号空间与落库 seq 本就分离，无功能影响）。
4. 迁移后的 fail-closed 诊断均保留"原因 + 结构化字段"，未出现吞异常；C3.1 的"零裸 eprintln"可机械复核（0 命中）。
5. 证据链完整：红相 Exit 101（真实断言失败，非编译/环境）→ 绿相 19 套件全绿 + `--lib` 1614 passed，`/tmp` 与仓库内 evidence 日志 md5 相同。
