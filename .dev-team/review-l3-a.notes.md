# L3-A 对抗审查（视角 A：并发/调度/状态流转/持久化一致性）

- 审查对象：`git diff c5761b27..HEAD`（HEAD 审时 `8afa4376`；审阅途中分支追加了 docs/evidence 提交，diff 由 51 → 55 文件，源码零变化，audited_scopes 已按 55 文件闭合）
- 契约真源：`.dev-team/contract-matrix-acp-delegate-p0p2.md`（C0–C6）；NFR：`.dev-team/nfr-baseline-acp-delegate-p0p2.json`
- 机读凭证：`PONY_BUILD_LOCK=… bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_delegate_persistence_contract` → `3 passed; 0 failed`，Exit 0（与 `docs/release/evidence/acp-delegate/c6-green.log` 一致）。C6.1/C6.2/C6.3 三红转绿真实成立，非空断言。
- 判定：**PASS**（0 BLOCKING / 5 ADVISORY）。无确定性缺陷；5 条均为「结构上成立、但当前生产者拓扑下不可确定性复现」的残余风险，交 Lead 裁决。
- ⚠️ 写域冲突提示：本波指定的写域路径 `.dev-team/review-l3-a.json` 在基线上是**另一波（ask-persistence，verdict FAIL）**的收据；已按任务书覆盖。原收据仍可从 `git show HEAD:.dev-team/review-l3-a.json` 取回，如需保真请 Lead 改名归档。

## 威胁面逐条回答

### 1) 并发 flush 重复写窗口（同 (session,turn) 双 flush 者）
- 终态路径 `persist`：`control_plane/mod.rs:1219`（锁内快照）→ `:1225`（释放锁后提交）→ `:1232`（成功后重新加锁移除）。**全程无 in-flight 标记**，安全性完全建立在 `:1175` 注释的「同 turn 单写者」假设上。
- 请求路径 `flush_requester`：`control_plane/mod.rs:1248` 持缓冲锁 → `:1260` `try_write` → `:1270` 提交 → `:1271` 移除。该路径**持锁跨越提交**，故自身无重复窗口。
- 结论：重复窗口只在「终态提交已释放 sessions 写锁」与「终态重新加锁移除」之间，且需 `flush_requester` 恰在该窗口取得缓冲锁并 `try_write` 成功；`flush_events_tx`（`sqlite_session.rs:1544`）按计数器分配 seq、**无 event_id 去重**，故真溢出即为「同内容两条不同 seq」。同一窗口的反向溢出是**事件丢失**（新入队事件随 `buf.remove` 被丢弃）。⇒ ADVISORY（finding[0]）。
- 现存生产者拓扑（已核）：ACP 子活动经 `emit_external_turn_event`（`turn_flow.rs:1494/1529`）在**父 turn 线程**同步执行（`orchestration.rs:214` `set_parent_turn` → `prompt` 内 `emit_update` 在调用线程，工具无并行分发），故未真正引入第二个同键写者。

### 2) 增量 flush vs checkpoint/watermark
- 每次 flush 都调 `finalize_event_watermark`（`control_plane/mod.rs:1425`），该函数会把**上一轮节点**reference 化：`store.rs:2633-2639` 写 `event_seq_range=(last_commit_watermark, new-1)` 并 `history.clear()/transcript.clear()/trace.clear()`；`last_commit_watermark` 在节点提交时取当时的 `event_watermark`（`store.rs:3721`）。
- 基线口径下该 finalize 只发生在「下一轮终态 flush」；本波改成每 hop 一次（`turn_stream.rs:847`、`turn_sync.rs:377`），于是上一轮节点的引用化区间终点**落在当前轮内部**，且快照/正文在**当前轮尚未提交**时即被丢弃。回读侧 `fold_node_views`（`store.rs:2647`）以 end 为界全量折叠 + 以「非祖先节点的 turn_id」排除（`projection.rs:1449-1481`），因此本轮最终提交后视图仍正确；**但本轮若崩溃/取消于 append_turn 之前**，旧节点视图只能由半截事件流重建。⇒ ADVISORY（finding[2]）。
- 崩溃一致性：C6.3 只要求「已 flush 事件可读 + turn 未完成」，实测成立；未发现事件与 turn 状态互相矛盾的确定性路径。

### 3) `emit_external_turn_event` 与父 turn 并发
- `sequence/event_id` 取 `next_turn_event_sequence`（`turn_flow.rs:861`，全局 `Mutex<HashMap<turn_id,_>>`）——**非 thread_local**，跨线程分配不冲突；父 turn 的事件序号会出现空洞（消费方按 store 的 `seq` 排序，无影响）。
- 归集事件 `is_terminal=false`（`turn_flow.rs:1529`）：不驱动 flush，靠父 turn 的 step 边界/终态 flush 落盘；`turn:completed` 之后 `clear_turn_event_sequence`（`turn_flow.rs:905`）再被外部通道取号会从 1 重发（event_id 撞号），但生产路径下 ACP 子活动全部发生在工具返回之前，终态 emit 在其后，不可达。
- 结论：未发现打乱父 turn 事件顺序或与终态 flush 交错的确定性路径。

### 4) 串行闸/取消/超时/资源
- `prompt_gate`（`acp/session.rs:891`）是无超时 `Mutex::lock()`：**NFR 声明的 `concurrency_lock.timeout_ms=600000` 无任何实现**；且 `acp_delegate` 每次调用新建会话（`orchestration.rs:214`），闸在实际拓扑下永不争用 —— 声明与实现双向脱节。⇒ ADVISORY（finding[1]）。
- 取消/预算：预算与取消仍由 session 单一裁决（`set_turn_budget` / `set_cancellation_token`），未新增第二套超时；`session.close()` 在正常与 invalid_arguments 早退路径均执行；未发现新增泄漏（子进程/pending permission 的回收语义未被本波改动）。
- flush 失败路径：contained（`turn_flow.rs:1587-1604` 只记日志返回 0），不阻断 turn、不丢缓冲（drain-on-success 保留）。

### 5) 写放大/锁持有/重试幂等
- `flush_requester` **持进程级缓冲锁跨越整批提交**（`control_plane/mod.rs:1248→1279`：SQLite 事务 + 逐事件 INSERT + 新增的 `normalized_sessions.created_at_ms` UPDATE `sqlite_session.rs:1569`），频率提到每 hop 一次 ⇒ 期间**全进程所有会话**的事件 emit 都被该互斥量挡住。⇒ ADVISORY（finding[3]）。
- 幂等性：drain-on-success（提交成功才移除）、`flush_events_tx` 的计数器与事件行同事务（`TEST_INJECT_FLUSH_FAILURE_DB_SUFFIX` 回滚点位于两者之间），失败重试不会产生重复 seq ⇒ **drain 语义真幂等**，未发现重试安全缺陷。

## 额外发现（finding[4]）
`eventTimes` 只有编码侧（`log_command.rs:240`），日志重放解码（`fold.rs:574-577`）不读，且 fold 对 `FlushEvents` 只计数（`fold.rs:857-861`）。日志是本仓声明的「真源」（ADR 0107 log-first），DB 由日志重建时 `created_at_ms` 就只能回落 flush 时刻 ⇒ C6.2 的时间语义在重建路径上不可复原（当前无消费者，故非 BLOCKING）。

## 已核对无问题的点（避免重复审）
- step 边界 flush 挂点顺序正确：先发下一跳 `calling_model`/StepStart，再 flush（`turn_stream.rs:812→847`）；`turn_sync.rs:377` 在批后、followup 前；`completed_hops` 仅在 `turn_stream.rs:146` 自增，pre/post 阶段 `completed_hops-1` 与 C4.1 同源。
- `event_times_ms` 与 `events` 平行、`sqlite_session.rs:1519` 用 `get(offset)` + `>0` 过滤，不会越界/写 0；`created_at_ms` 的 `MIN(...)` 只前移不覆盖；实测 C6.2 断言走的是非 NULL 分支（重跑 `--nocapture` 无 “IS NULL” 打印）。
- `C1.2`：ACP 文本 chunk 只进 `NoopTurnEventSink`（`orchestration.rs:1313`），不入父正文。
- `C0.1–C0.4`：`result.rs` 归约按到达序、超限截断单一常量；`Truncated/Refused/Cancelled/Error` 不带 `finalText`。
- `C5.1`：`ProviderCallRetryMeta::from_window` 恒 `attempts≥1`、另两键仅重试时 `Some(>0)`；`projection.rs:1310-1345` 由 ProviderUsage 重建的记录 `attempts=None`（旧记录语义，契约注释已豁免）。
