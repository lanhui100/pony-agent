# F2 / F3 实施收据（Executor: exec-core）

- 工单：共享任务板 `task-5`（F2 重试取样窗口纯净）、`task-6`（F3 节流离开 SSE 读线程）
- 时间窗：2026-11-12 19:37–20:00 (+08:00)
- 环境纪律：工作区既有 47 项**无关**未提交改动全量保留；本单只做增量最小改动，
  未 `git commit`/`stash`/`checkout`，未改任何 `tests/`、`turn_flow.rs`、
  `metrics_snapshot.rs`、`otel/mod.rs`、`bin/tui.rs`、前端。
- 写域实际触及：`agent/retry.rs`、`agent/provider/mod.rs`（F2）、
  `agent/runtime/turn_stream.rs`、`agent/runtime/tool_recovery.rs`（F3 + F2 调用点）。

---

## 一、F2 取证（I1）——结论：**成立**（`dur` 跨 attempt 累加；`gen` 因两端同源近似抵消，`end_to_end` 口径被系统性压低）

### 1. 取样点（全部落在重试循环**之外**，行号为改动前）

| 路径 | dur 取样点 | ttft 取样点 | 说明 |
|---|---|---|---|
| 初始流式决策 | `turn_stream.rs:2174` `let initial_decision_started_at = Instant::now();` → `:2342` `initial_decision_started_at.elapsed()` | `:2244` / `:2250` `initial_decision_started_at.elapsed()` | 起点在 `provider_decision_stream_guarded_with_mode`（`:2196`）**之前** |
| 初始同步（buffered）决策 | `:2395` `let started_at = Instant::now();` → `:2404` | 无（`BufferedResponse`，ttft=None） | 无 ttft ⇒ 分母只能用 `dur`（`end_to_end`） |
| 工具 followup 流式 | `:978` `provider_call_started_at = Instant::now();` → `:1164` | `:998` / `:1039` | 起点在 `provider_followup_stream_with_calls`（`:981`）之前 |
| 工具恢复（同步/流式） | `tool_recovery.rs:19` / `:103` → `:33` / `:279` | `:143`（相对 `started_at_for_emit`） | 同上 |

### 2. 重试循环（在取样窗口**内部**）

- 唯一生产重试循环：`provider/mod.rs:5682-5704`（`retry_provider_scoped_with_report_config`）。
  退避 sleep 在 `:5692`（`sleeper.sleep(delay)`），累计进 `slept_ms`（`:5700-5702`）。
- followup 撞限流时切长退避：`provider/mod.rs:5436-5445`（20s→40s，budget 90s）。
- 单次失败 attempt 的实测耗时 1.9–8.0s：`.dev-team/diag-tps/pony/report.md` 附录 E.3。
- 重试触发时 `on_delta` 尚未发出任何增量（`mod.rs:1748-1754`
  `stream already emitted delta; auto-retry forbidden`）⇒ 失败尝试必然发生在首 token 之前。

### 3. 因果判定（**关键分寸**）

`dur` 与 `ttft` 取自**同一个 Instant**，所以：
- `gen = dur − ttft`：失败尝试在分子分母两端同时出现 ⇒ **近似抵消**（物理探针实测
  `gen_outer = 400ms == gen_pure = 400ms`，见 §五 P1）；
- `dur` **单独**当分母的 `end_to_end` 回退口径（无 ttft 的 buffered / 恢复调用、
  以及 `gen < max(200ms, 5%dur)` 被门槛剔除后的回退）⇒ 失败尝试与退避被当成生成
  时间，**系统性压低**：物理探针同场景 `dur` 2600ms vs 纯净 500ms（**5.2×**）；
  现场同批 2903 条真实记录复算 `e2e 10.4 t/s vs decode 98.6 t/s`（diag C6/E.3）。

⇒ 依 I2 修：`gen` 只覆盖最后一次成功 attempt；`attempts` 次数与失败耗时改为**独立字段**（结构化日志），绝不混入 `gen`。

---

## 二、F2 改动（I2/I3）

### 1. `agent/retry.rs:1149-1395` 新增「重试账本」（纯记账，**不算速度**）

- `ProviderAttemptWindow`（`:1171`）：`attempts` / `failed_attempt_ms` / `slept_ms` /
  `succeeded_attempt_ms`（＝最后一次成功 attempt 的耗时）/ `succeeded_attempt_offset_ms`
  （成功 attempt 起点相对账本起点的偏移）。`prelude_ms()`、`rebase_duration_ms()`、
  `rebase_first_token_ms()` 三个纯函数即「外层观测 → 纯净窗口」的唯一换算出口。
- 线程本地账本（`thread_local! ATTEMPT_WINDOW`，`:1246` 附近）：一次 provider 调用
  （含其内部重试/换手）在**同一线程同步执行**，与 `runtime/limits.rs:64-68` 同一取舍。
- API：`begin_provider_attempt_window()`（`:1253`，**返回与调用方计时同源的 Instant**）、
  `note_provider_attempt_failed / _succeeded / _backoff_slept`（`:1265`/`:1278`/`:1294`）、
  `take_provider_attempt_window()`（`:1260`）、
  `take_pure_call_window(outer_started_at, outer_ttft)`（`:1330`）。
- I2 的「独立字段」载体：`log_pure_call_window`（`:1355`）在**发生过重试**时打一行
  `{"message":"provider_call_retry_window", attempts, failed_attempt_ms, slept_ms,
  prelude_ms, succeeded_attempt_offset_ms, pure_duration_ms, pure_first_token_latency_ms,
  outer_duration_ms, outer_first_token_latency_ms}`（无重试不写，避免常态噪声）。
  **未新增任何 wire 字段 / DB 列**（Lead 裁决）。
- I3：本模块**没有**任何速度公式（无除法、无 `tok/s`），速度唯一出口仍是
  `turn_flow::{decode_eligible, call_speed_from_raw}`（未改动）。

### 2. `agent/provider/mod.rs:5703-5724` 重试循环逐 attempt 记账

```rust
            // F2（重试窗口纯净）：退避 sleep 是「成功 attempt 之前」的耗时，逐次记账。
            crate::agent::retry::note_provider_attempt_backoff_slept(slept);
        }
        // F2：逐 attempt 计时——失败耗时与成功 attempt 的窗口在这里各自落账
        let attempt_started_at = Instant::now();
        let attempt_result = operation();
        let attempt_elapsed_ms = attempt_started_at.elapsed().as_millis() as u64;
        match attempt_result {
            Ok(value) => {
                crate::agent::retry::note_provider_attempt_succeeded(
                    attempt_started_at,
                    attempt_elapsed_ms,
                );
                ...
            }
            Err(err) => {
                attempts_total += 1;
                crate::agent::retry::note_provider_attempt_failed(attempt_elapsed_ms);
```

调度语义零变化（预算/sleep/分类/返回文案全部原样）；账本未开启时三个 `note_*`
是静默 no-op，第三方直接调用 provider 的行为不受影响。

### 3. 调用点（4 处，每处 2 行）

| 调用点 | begin | 结算 |
|---|---|---|
| `turn_stream.rs:2302`（初始流式决策） | `initial_decision_started_at = begin_provider_attempt_window()` | `:2474 take_pure_call_window(...)` → `dur = pure.duration_ms`、per-call ttft = `pure.first_token_latency_ms`（轮级 TTFT 口径不变） |
| `turn_stream.rs:2536`（初始 buffered 决策） | `started_at = begin_...()` | `:2544 take_pure_call_window(started_at, None)` |
| `turn_stream.rs:1094`（followup 流式） | `provider_call_started_at = begin_...()` | `:1285 take_pure_call_window(...)` → 记录用纯净 dur/ttft |
| `tool_recovery.rs:25` / `:111`（恢复同步/流式） | 同上 | `:40` / `:281` 同上 |

语义：`gen` 只覆盖最后一次成功 attempt；**无成功 attempt**（整体失败）时
`rebase_*` 原样返回外层值（不伪造纯净性、不回填 0）。

---

## 三、F3 取证与改动（I1/I2/I3）

### 1. 取证

- `on_delta` **在读循环内联执行**：`provider/openai_sse.rs:85-161`（`push_payload`）
  在 `:133`（Text）/`:139`（Reasoning）/`:149`（StreamStart）同步调 `on_delta`。
- 旧实现 6 处「睡眠到 16ms 间隔」节流：`turn_stream.rs:1078,1158,2283,2333`、
  `tool_recovery.rs:214,244`（改动前行号）——睡眠期间停止读 socket ⇒ TCP 背压。

### 2. 改动：`StreamDeltaCoalescer`（`turn_stream.rs:10-215`）

- 结构：`CoalescedFrame`（单帧载荷）／`CoalescedBatch`（一个窗口攒下的内容，
  reasoning 与 text **各一槽**）／`StreamDeltaCoalescer`（`window_ms = 16`）。
  `push()` 只做字符串拼接 + 取首个 latency；`take_if_due(now_ms)` 只在窗口到点/首个
  增量时返回批次；`flush(now_ms)` 收尾无视窗口发完。
  **无 `thread::sleep`、无等待、无有界队列、无独立线程**。
- 帧形状**与旧实现逐字一致**：`CoalescedBatch::into_frames()` 拆成「先 reasoning 帧、
  后 text 帧」（旧实现同样是先 flush 思考再 flush 正文，两帧同刻、中间不睡眠），
  合并只发生在**同类型**之内。单个窗口上屏帧数上限仍是 2（与旧实现相同）。
  这一条是为守住既有契约：`runtime/tests.rs:6956`
  （`start_turn_stream_emits_first_token_latency_on_reasoning_delta`）要求
  「首个 delta 只带 reasoning、text 为 None」——初版把两类内容揉成一帧曾使该用例失败，
  已按上述分槽方案收敛并复跑至绿（见 §四 V6/V7）。
- 读线程耗时有上界（I3）：每个 delta = 1 次拼接 + 1 次时间戳比较（毫秒级整型比较），
  **不随客户端消费速度增长**。
- 收尾必发（I2）：两处流式路径在 provider 调用返回后**无条件** `flush()` 上屏
  （`turn_stream.rs` followup/initial 两条流的 tail 块、`tool_recovery.rs` 恢复流 tail 块）；
  终态（`turn:completed` / `turn:failed` / `turn:cancelled`）本就不经此路径，未触碰。
- 首字延迟语义保持：首个内容增量一定落在**空窗口**（`last_emit_ms = None`）⇒ 立即上屏，
  首帧仍携带 `first_token_latency_ms`；同批次两帧共用同一个首字延迟值（旧实现同样）。
- sink 契约（Lead 修订 v2 ①）：`turn_stream.rs:11-33` 文件头写明
  「`sink.emit` 内联且契约=非阻塞投递（Tauri fire-and-forget / TUI 无界 channel /
  headless 无消费者）；慢 sink 会串进读循环，属**已知限制**，必须在 sink 实现侧解决，
  本聚合器永不阻塞读线程」，并在 3 处 sink 调用点各留一行同口径注释。

### 3. 6 处 sleep 全部消除（机械验收 ②）

```console
$ grep -rniE "MIN_CLIENT_GAP|thread::sleep" \
    crates/pony-agent-core/src/agent/runtime/{turn_stream.rs,tool_recovery.rs}
（无匹配）
GREP_EXIT=1   # grep 无匹配 ⇒ 6 处全部消除
```

---

## 四、验证（物理收据）

| # | 命令 | Exit | 关键输出 |
|---|---|---|---|
| V1 | `cargo check -p pony-agent-core` | **0** | `Finished dev profile`（仅有 1 条**改动前既有** warning：`provider/mod.rs:37 unused import: extract_chat_delta_text`） |
| V2 | `cargo test -p pony-agent-core --test pa172_provider_failover_contract` | **0** | `11 passed; 0 failed` |
| V3 | `cargo test -p pony-agent-core --test tui_metrics_snapshot` | **0** | `133 passed; 0 failed` |
| V4 | `grep -rniE "MIN_CLIENT_GAP\|thread::sleep" …/{turn_stream,tool_recovery}.rs` | 1（无匹配） | 见 §三.3 |
| V5 | F2 物理探针（见 §五） | **0** | `PROBE_OK`（4 条断言全过） |
| V6 | `cargo test -p pony-agent-core --lib`（全量单测，非交付门禁，用于确认**无流式回归**） | 101 | `1525 passed; 2 failed` —— 两条失败**均与 F2/F3 无关**，见下 |
| V7 | `cargo test -p pony-agent-core --lib runtime::tests::start_turn_stream_emits_first_token_latency_on_reasoning_delta` | **0** | `1 passed`（曾因初版把 reasoning+text 揉成一帧而失败，改分槽后转绿） |

> 说明 1：V1 曾在 19:44 前后读到一次 `E0425: turn_call_census not found`
> （`metrics_snapshot.rs`）——那是 F1（`task-4`，他人写域）**编辑中途**的瞬时状态，
> 与我无关；其落定后 V1 稳定 Exit 0。
>
> 说明 2：V6 的两条失败是 **F1（`turn/speed` 新事件）与冻结用例的冲突**，证据：
> - 失败断言 = `runtime/tests.rs:4267`「completed skeleton shape」与
>   `control_plane/mod.rs:8063`「event stream persisted end to end」，两侧差异**只**是
>   多了一条 `turn/speed`；该用例文件 `git diff --stat` 为空（HEAD 就期望**没有** `turn/speed`）。
> - `settle_turn_speed_evidence` 的派发点在 `turn_flow.rs:1033`（F1 写域）；
>   sync 与 stream 两条路径**都**走了它（日志两条 turn 都打了
>   `"message":"turn speed settled"`），即多出的那条事件来自 F1 的新 I1 实现。
> - 本单改动只影响 per-call 的 `dur`/`ttft` **取值**与 delta 帧的**形状/时机**，
>   不涉及任何 `TurnEvent::TurnSpeed` 的构造或派发。
> ⇒ 已在本单外上报 Lead：这条应由 F1 决定「补 Test Agent 期望」或「按契约收敛事件面」。

---

## 五、F2 物理探针（不改仓库，只读链接已构建 rlib，产物落 `/tmp`）

场景复刻现场：attempt#1 撞 429 耗时 1900ms → 限流退避 200ms（真实配置 20s，探针缩短）
→ attempt#2 成功耗时 500ms，其中 100ms 后首 token。

```console
$ rustc --edition 2021 -O /tmp/f2_window_probe.rs \
    -L dependency=target/debug/deps \
    --extern pony_agent_core=target/debug/libpony_agent_core.rlib -o /tmp/f2_window_probe
RUSTC_EXIT=0
$ /tmp/f2_window_probe
{"attempts":2,"failed_attempt_ms":1900,"level":"info","message":"provider_call_retry_window",
 "outer_duration_ms":2600,"outer_first_token_latency_ms":2200,"prelude_ms":2100,
 "pure_duration_ms":500,"pure_first_token_latency_ms":100,"slept_ms":200,
 "succeeded_attempt_offset_ms":2100,"trace_id":null,"ts":"2026-11-12T11:54:46.690Z"}
outer: dur=2600ms ttft=Some(2200)  =>  gen_outer=400ms
pure : dur=500ms  ttft=Some(100)   =>  gen_pure=400ms
ledger: attempts=2 failed_attempt_ms=1900 slept_ms=200 prelude_ms=2100 \
        succeeded_offset_ms=Some(2100) retried=true
PROBE_OK
PROBE_EXIT=0
```

读法：
- `dur` 从 2600ms 收敛到 **500ms**（只含成功 attempt）⇒ `end_to_end` 分母不再是 5.2× 虚高；
- `gen` 400ms 两端一致 ⇒ 证明「`dur−ttft` 近似抵消」的诊断成立，修复不改变 decode 口径的既有语义；
- `attempts=2 / failed_attempt_ms=1900 / slept_ms=200` 是**独立字段**（结构化日志），
  未进入 `dur`/`gen`；
- 探针内 4 条 `assert_eq!` 全过（`PROBE_OK`）——即 Test Agent 的
  `tests/f2_retry_window_purity.rs` 可用同一公共 API（`agent::retry::*`，已 `pub`）
  写等价断言。

---

## 六、取舍与风险

1. **F3 合并策略 vs UI 平滑度**：上屏帧率上限仍是 62.5 帧/s（窗口 16ms 不变），
   变的是「窗口内不再阻塞读线程」，而是把同一窗口的 text/reasoning **合并成一帧**。
   观感上每帧内容略变粗（最多 16ms 的 token），帧率不降；突发路由下 token 率不再被
   62.5/s 掐死（读线程不停读 socket）。同一 delta 内的 reasoning+text 由「两帧」变
   「一帧双字段」，消费端本就按字段独立渲染，无顺序依赖。
2. **紧邻的上屏延迟**：pending 最多等到「下一个 delta」或收尾 `flush`，即最坏退化为
   一个 chunk 间隔（现状是 21–43ms 稳态），不引入额外固定延迟。
3. **F2 的账本载体是 thread_local**：与 `runtime/limits.rs` 的既有取舍一致（turn 在
   调用线程同步执行）。若将 provider 调用挪到别的线程再取账本会读到 `None` ⇒
   自动退回外层观测（fail-safe，不伪造纯净性）。
4. **F2 覆盖面残差（不在我的写域，需 Lead 裁决）**：以下 3 处仍在外层取样，
   未纳入本次修复（同样可用 2 行 `begin/take` 收口）：
   - `runtime/turn_sync.rs:375` → `:412`（同步 followup per-call dur）
   - `runtime/turn_prep.rs:186-199`（同步首跳 decision duration，`turn_prep.rs` 亦非我写域）
   - 反例：`turn_flow.rs:2858-2860` 的 `llm.chat` span 用 `elapsed_ms` 记 per-call 耗时，
     同样是跨 attempt 值；该文件是 F1 写域，**未动**。
   ⇒ 影响：TUI/桌面 t/s 的 per-call 记录（snapshot 口径）已纯净；OTel `llm.chat` span
   的 per-call dur 仍含重试（可观测性残差，不影响 speed 公式）。
5. **日志量**：`provider_call_retry_window` 仅在发生重试时写一行（每调用至多 1 行），
   相对现有「每 chunk 一行 `usage:openai raw=null`」可忽略。

---

## 七、遗留 / 建议

- Test Agent 的 `tests/f2_retry_window_purity.rs` 建议用公共 API 断言三件事：
  (a) `rebase_duration_ms` 只返回成功 attempt；(b) `rebase_first_token_ms` 扣除
  `succeeded_attempt_offset_ms`；(c) 无成功 attempt 时原样返回外层值（不伪造）。
- `tests/f3_stream_throttle_isolation.rs` 建议断言：`on_delta` 处理 N 个 0ms 间隔增量
  的总耗时 ≈ O(N)（无 16ms×N 累加），且收尾 flush 后正文完整（禁止丢最终帧）。

---

## 八、F2 残差补齐（Lead 批准扩写域，2026-11-12 20:0x）

批准范围（最小改动，每处 ≤2 行，无重构）：
- `runtime/turn_sync.rs:375→:412`（同步 followup per-call dur）
- `runtime/turn_prep.rs:186-199`（同步首跳 decision duration，两个分支各一处）

改法与本单 §二 完全一致：`begin_provider_attempt_window()` 取代 `Instant::now()`（返回与
外层计时同源的起点），返回后 `take_pure_call_window(<起点>, None).duration_ms` 取代
`<起点>.elapsed()`（同步/buffered 无首包观测 ⇒ ttft 传 `None`，不伪造）。

不在本单（Lead 已派给 exec-metrics）：`turn_flow.rs:2858-2860`（`llm.chat` span per-call dur）。

### 追加物理收据（全部 Exit 0）

| # | 命令 | Exit | 关键输出 |
|---|---|---|---|
| V8 | `cargo check -p pony-agent-core` | **0** | 0 error |
| V9 | `cargo test -p pony-agent-core --lib` | 101 | `1525 passed; 2 failed` —— 仍是 F1 `turn/speed` 与冻结用例的冲突（`runtime/tests.rs:4267` / `control_plane/mod.rs:8063`），已由 Lead 派给 test-agent 补期望；**本单新增的 turn_sync/turn_prep 改动零新增失败** |
| V10 | `cargo test -p pony-agent-core --test f2_retry_window_purity` | **0** | `2 passed`（真 mock HTTP：503 失败 700ms → 重试成功；断言 per-call 窗口不含失败 attempt，且 attempts/failed 独立字段可读） |
| V11 | `cargo test -p pony-agent-core --test f3_stream_throttle_isolation` | **0** | `2 passed; 1 ignored`（ignored = Lead 修订 v2 降级的慢 sink ADVISORY） |
| V12 | `cargo test -p pony-agent-core --test f1_speed_observability` | **0** | `4 passed` |
| V13 | `cargo test -p pony-agent-core --test pa172_provider_failover_contract` | **0** | `11 passed` |

> V9 的 2 条失败在补齐前后**完全同形**（左/右差异只多一条 `turn/speed`），且用例文件
> `git diff` 为空 ⇒ 与本单改动无因果。
