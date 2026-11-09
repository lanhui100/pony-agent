# PA-171 契约矩阵（wave-68）：goal 续轮失败止损 / 失败不烧预算 / 限流冷却 / 失败轮归因

Amendment: wave-68-a1（2026-11-09）

> - **a1-1** §4.2 / C4-S5：`fallback_reason` 封闭值域（`provider_decision_failed` 首跳 / `provider_followup_failed` 工具 followup / `turn_failed_no_fallback` 默认值 + `provider_meta=None` 缺席态）、判定优先级、附件保存失败走默认值的例外。
> - **a1-2** §6.1：`provider_cooldown_entered` / `provider_cooldown_short_circuit` / `goal_rounds_stopped` 的契约归属与封闭字段；metrics 结构性 N-A ⇒ 结构化日志是唯一观测面。
> - **a1-3** §3.1 / C1-A4：`start_turn_stream_checked` outcome 派生判别式（`Fail⇒Failed` / `Cancel⇒Cancelled` / `WaitUser⇒Suspended` / 其余按 phase），并闭合 C1-A2 的 error 采集条件。
> - **a1-4** §3.3 / C3-P7：冷却键改为**四段** `requested_name|provider_name|model|base_url`（**D68-7**，Lead 批准；理由：同进程内不同 endpoint 各有独立配额窗口，三段键会把 C3-d 的 503 反向钉子用例短路成「冷却中」，生产上造成跨环境误杀）。
> - **a1-5 已知限制（ADVISORY，如实登记，未接线）**：`clear_cooldown()`（`crates/pony-agent-core/src/agent/provider/mod.rs:737`）与 `cooldown_remaining()`（`:732`，仅被 `ensure_not_cooling` 在 `:743` 内部消费）**当前零生产调用点** ⇒ 冷却期内没有任何用户/宿主可操作的复位入口，**复位只能靠进程重启**；单条冷却由 6h 冻结上界兜底（§3.3 C3-P5）。本项**不**作为「已接线」宣称。

Amendment: wave-68-a2（2026-11-09，Phase-1 计划审查整改）

> - **a2-1** `nfr-baseline-wave68.json` → `retry` **分层化**：删除歧义的顶层 `backoff_cap_ms` / `backoff_ms`，改为 `decision_rate_limited_max_attempts=1`（限流/配额首次即 Abort）、`decision_other_transient_max_attempts=5`、`decision_timeout_backoff{500,×2,8000,30000}`、`followup_rate_limit{3 次,20s/40s,90000}`；保留 `max_attempts`（门禁 schema 需要）+ `max_attempts_note` 声明「仅供旧消费者，真值见分层字段」。
> - **a2-2** `nfr-baseline-wave68.json` → 新增 `wave68_constants`：冷却键**四段**（D68-7）、`default_window_ms=60000`、`max_window_ms=21600000`、`registry_max_entries=64`、`goal_round_failure_stop.max_consecutive_failures=2`、`fallback_reason` 封闭值域 —— 使**矩阵 / NFR / 代码三处同源**（`provider/mod.rs:615,617,619,721-729`、`goal.rs:404`）。
> - **a2-3** §4.2 / C4-S5：明确 `fallbackReason` 属**既有字段语义变更（向后兼容但非 additive，`None` → 三字面量）**，C4 不再套用「additive only」；并补优先级真值表示例 E1–E5（含附件保存失败走默认值的例外）。
> - **a2-4** §2.2 / C1-S3 + 新增 C1-S5：`GoalRoundStopReason` 实际六变体，补 `TurnSuspended` / `TurnUnclassified` 的逐字文案与触发映射（含 **`Ok(Cancelled) = Cancelled`**、`Err(_) ⇒ TurnUnclassified`，不计失败不烧预算）；另加 C1-T3′ 要求**每个非成功 `break` 都带可见原因**。

Amendment: wave-68-a3（2026-11-09，方案复审 round2 整改 · 契约/NFR 与实现对齐）

> - **a3-1** §2.3 / C1-T2 改写为实现真值：租约是 **`Arc<Mutex<HashSet<String>>>` 按 `session_id` 分区**（`tui.rs:8331`、`8339-8345`、`8362-8369`），未抢到时**可见上报** `LoopAlreadyOwned`（`tui.rs:8164-8179`、`:8170-8177`），不再写「`Arc<AtomicBool>` + 静默 return」。
> - **a3-2** `nfr-baseline-wave68.json` → `wave68_locks.goal_round_loop_lease` 与 a3-1 同构改写（另补 `critical_section` / `forbidden_under_lock` / `poison_recovery` / `no_await_across_lock`）。
> - **a3-3** `LoopAlreadyOwned` 登记：§2.2 C1-S3 增第七行（逐字文案取 `goal.rs:470-472`）+ NFR `visible_stop_reason_variants` 增该项。
> - **a3-4** 租约**零机械测试锚**如实登记为遗留缺口（`tests/` 全目录 grep 0 命中），不伪称已有锚点。

> **批准声明（Protocol Agent, 2026-11-09）**：本矩阵**批准** Test Agent 已冻结并锚定的红相面
> （`crates/pony-agent-core/tests/pa171_goal_round_failure_contract.rs`、
> `pa171_provider_rate_limit_cooldown_contract.rs`、
> `pa171_failed_turn_attribution_contract.rs`、
> `pa171_error_copy_contract.rs`）。§2–§5 的「冻结面」逐字抄录自上述文件的 `assumed_contract_surface`
> 文档块（含签名、变体、常量值），其后为**语义约束补齐**。
> **冲突裁决规则**：签名/命名以本矩阵为准；**断言语义不得放宽**——任何实现选择都不得让红相断言变弱
> （连续 2 轮失败即停 / 失败轮不推进预算 / 冷却窗口内 0 次 HTTP / 三字段非占位 + error_class 可判）。
> Test Agent 文件里「若矩阵给出不同签名，请 Lead 裁决后同步本文件**调用点**」的授权在此生效：
> t-red 只允许改**调用点拼写**，不得改断言。
>
> 基线：主工作树 `/home/dm/pony-agent`，`git rev-parse --short HEAD` = **`d9425a70`**（与
> `.dev-team/state.json` → `waves[wave=68].baseline_commit` 一致）。所有 `path:line` 以该基线为准。
> 标记约定：**事实** = 源码/命令直接取证；`[DERIVED]` = 由事实推导的契约选择；`[UNVERIFIED]` = 未独立复核。

---

## 0. 事实基线（本次冻结前已取证）

| 编号 | 结论 | 证据（path:line） |
|---|---|---|
| F1 | goal 续轮循环不看上一轮成败，只判 Esc / 8 轮 / 待作答请求 | `crates/pony-agent-core/src/bin/tui.rs:8136-8140`（`should_abort_goal_rounds` + `session_has_pending_host_requests` 为唯一停止判据）、`:767-769` |
| F2 | 失败轮照样推进预算：`advance_goal_round` 在**起轮之前**调用，且无条件 `+1` | `tui.rs:8149-8153`、`crates/pony-agent-core/src/agent/goal.rs:380`（`rounds_started.saturating_add(1)`）、`:361-397` |
| F3 | 预算耗尽分支是**静默 break**（无任何用户可见原因） | `tui.rs:8146-8148` |
| F4 | 宿主入口丢弃 `Result`，宿主拿不到轮次 outcome | `crates/pony-agent-core/src/agent/control_plane/mod.rs:2139-2146`（`let _ = self.start_ask_aware_turn_stream(...)`） |
| F5 | 轮次终态相位可从事件采集：`RecordingTurnEventSink` 只在 `turn:completed/failed/cancelled/suspended` 记终态，`build_turn_result().phase` 直接来自该终态 | `control_plane/mod.rs:881-891`、`:763-788`（`phase = terminal.phase.clone()?`）、`crates/pony-agent-core/src/agent/turn_flow.rs:573`（`failed`）、`:639`/`:760`（`cancelled`）、`crates/pony-agent-core/src/agent/runtime/types.rs:313`（`SUSPENDED_TURN_PHASE`） |
| F6 | 决策侧 phase 与 graph 决策可判：`decision.kind`（Fail/Cancel/WaitUser）+ `turn_result.phase` | `control_plane/mod.rs:2025-2035`（`decide_graph_after_turn_with_planner`）、`:2009-2024`（suspended 分支）、`crates/pony-agent-core/src/agent/graph.rs:829-836`（`GraphDecisionKind::Cancel` 等） |
| F7 | 限流/配额走**短退避 ×5**（Decision 作用域），`Retry-After` 是死码 | `crates/pony-agent-core/src/agent/provider/mod.rs:4401`（`PROVIDER_TIMEOUT_RETRY_MAX_ATTEMPTS = 5`）、`:4505-4514`（500ms→8s / 30s budget）、`:4677-4680`（长退避门禁仅 `Followup`）、`crates/pony-agent-core/src/agent/retry.rs:270-293`（`RetryAfter` 零生产调用点） |
| F8 | 4 个 HTTP 出口，非 2xx 各自内联拼错误文本（无 `Retry-After` 读取） | `provider/mod.rs:2106-2145`（`post_openai_stream_response`，错误在 `:2137-2142`）、`:2150-2199`（anthropic 流式，`:2190-2195`）、`:2520-2559`（`post_openai_json`，`:2543-2548`）、`:2561-2600`（`post_anthropic_json`，`:2593-2600`） |
| F9 | 限流码表唯一来源为 `RATE_LIMIT_LONG_BACKOFF_HINTS`（含 `quota`，**不含** `usage limit` / `allowance`） | `retry.rs:344-352`、`is_rate_limit_error` 于 `:746-752` |
| F10 | `ErrorCode::UpstreamQuota` 已存在且 `upstream_code_for_hints` **从不产出**它（quota-only 落 `None` ⇒ NonRetryable Abort） | `crates/pony-agent-core/src/agent/error_code.rs:188`（wire `upstream_quota`）、`retry.rs:306-312`（七词注释）、`:576-611`（`upstream_code_for_hints` 无 quota 分支） |
| F11 | 失败轮三字段硬编码 `None`；失败 `TurnResult` 同一语义位置用 `"failed"` | `turn_flow.rs:581-583`（`provider_source: None` / `provider_mode: None` / `fallback_reason: None`）、`:455-457`（`TurnResult.provider_source = "failed"`、`provider_mode = "failed"`） |
| F12 | 失败落盘链路另有两处传 `None`：checkpoint 与 trace record | `crates/pony-agent-core/src/agent/runtime/turn_persist.rs:461-475`（`update_execution_checkpoint(... None, None, None ...)`）、`:476-502`（`provider_source=None, provider_mode=None, ... fallback_reason=None`）、`:805-806`/`:810`（record 字段）、`:888-906`（checkpoint 形参） |
| F13 | TUI 把一切瞬态错误（含 429）折叠成「重试已用尽」 | `crates/pony-agent-core/src/bin/tui/errors.rs:76`、`:137-151`（`is_transient_provider_error` 分支在 `:146`） |
| F14 | 首轮与续轮都在**轮次线程内**同步跑；每个用户提交各起一个轮次线程 ⇒ 多个循环实例可并存 | `tui.rs:9756-9790`（`thread::spawn` + `start_turn_stream` + `run_goal_rounds`）、`:10215-10240`（Ask 续轮线程） |
| F15 | 结构化观测的四维归属真值源 | `docs/observability.yaml:14-65`（dimensions）、`:69-91`（enforcement_points） |

`[UNVERIFIED]` 项：**U1** 是否有 Err 路径既不派发终态又不清 `is_streaming`（既有隐患，本卡非目标）；
**U2** 各家配额错误正文是否恒带恢复时间（本契约按「可解析则显示、不可解析则如实说未知」处理）。

---

## 1. 全局不变量（G1–G7，破一条即判未过）

| 编号 | 不变量 | 证据/理由 |
|---|---|---|
| G1 | **单一码表**：限流/配额/超时/认证/网络的判定只允许扩展 `retry.rs` 既有常量与 `ErrorCode`，禁止在任何文件内新增第二套关键词表或第二套错误码字面量 | `retry.rs:296-352`（码表唯一真源）、`error_code.rs:1-15`（「只新增不改旧分支」）；C4 的 `error_class` 必须与 `ErrorCode` wire 词表对齐（`timeout` 即 `UpstreamTimeout` 的 wire 码，`error_code.rs:184`） |
| G2 | **additive only**：所有新增字段 `#[serde(default)]` + `skip_serializing_if`（可选字段），旧消费者不读即行为不变；禁止改既有字段名/类型/语义 | `runtime/types.rs:141-215`（`rename_all = "camelCase"` 既有约定）、`:196-199`（`stop_reason` additive 范式） |
| G3 | **既有语义冻结**（不得改）：`should_abort_goal_rounds`、`goal_rounds_must_yield_to_pending_requests`、`ProviderRetryPolicy::classify`/`decide`、`RetryBudget`、`advance_goal_round` 的 CAS/持久化（含回滚分支）、`ERROR_SUMMARY_TRANSIENT` 逐字文案、`LEGACY_FAILED_TURN_TEXT`、`emit_stream_failed` 14 参签名 | `tui.rs:767-769`、`:777-783`（被 `tests/tui_batch2_regression.rs:37-351` 冻结）、`retry.rs:623-707`、`goal.rs:361-397`、`turn_flow.rs:489`、`:532-549` |
| G4 | **无新增外部调用面**：不联网、不起线程、不新增定时器；冷却判定纯内存 | 本卡非目标「跨 provider 自动故障转移」「重试框架重构」（任务卡 §非目标） |
| G5 | **可测性**：所有新判定先落纯函数（时钟/`now` 一律作为入参或可注入），测试禁止 `sleep` 等待真实窗口 | 红相 C3-c 用同一 `ProviderManager` 实例连续两次调用验证，不 sleep |
| G6 | **失败语义不变**：`kind == "failed"`、`error` 原样保留、`text` 文案不变、PonySentry 上报保留 | `turn_flow.rs:552-556`、`:568`、`:576`；红相 C4-a 显式断言 |
| G7 | **不静默**：任何新增停止路径都必须产出可见状态行 + 结构化日志，禁止裸 `break` | F3 为历史缺陷；红相 C1/C2 的 `visible_line()` 断言 |

---

## 2. C1 + C2：续轮失败止损与失败不烧预算（红相：`tests/pa171_goal_round_failure_contract.rs`）

### 2.1 冻结面（**逐字**抄录自 t-red 文件 `:16-39`，写域 `agent/goal` = `t-w1`）

```rust,ignore
pub const MAX_CONSECUTIVE_GOAL_ROUND_FAILURES: u32 = 2;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalRoundOutcome { Succeeded, Failed }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRoundStopReason {
    Cancelled,
    ConsecutiveFailures { failures: u32 },
    BudgetExhausted { rounds_started: u32, max_rounds: u32 },
}
pub fn goal_round_stop_reason(
    cancelled: bool,
    consecutive_failures: u32,
    rounds_started: u32,
    max_rounds: u32,
) -> Option<GoalRoundStopReason>;              // Some ⇒ 停止新增 goal 轮（不静默 break）
pub fn advance_goal_round_for_outcome(
    goal_id: &str,
    outcome: GoalRoundOutcome,
) -> Result<GoalData, String>;                 // 仅成功轮 rounds_started += 1
impl GoalRoundStopReason {
    pub fn visible_line(&self) -> String;      // TUI 可见状态行（含停止原因）
}
```

红相另冻结的导入面（`tests/pa171_goal_round_failure_contract.rs:50-54`）：`agent::goal` 必须 `pub` 再导出
`advance_goal_round_for_outcome`、`create_goal`、`goal_round_stop_reason`、`reset_goal_store_for_test`、
`CreateGoalArgs`、`GoalRoundOutcome`、`GoalRoundStopReason`、`GOAL_STORE_PATH_ENV`、
`MAX_CONSECUTIVE_GOAL_ROUND_FAILURES`（既有项见 `goal.rs:355-361`、`goal.rs` 常量与 `CreateGoalArgs:43-49`）。

### 2.2 语义约束（补齐）

- **C1-S1 `goal_round_stop_reason` 判定优先级（冻结）**：`cancelled` → `ConsecutiveFailures` →
  `BudgetExhausted` → `None`。`ConsecutiveFailures` 的 `failures` 取**入参原值**（不得截断为 2；
  红相 `:154-162` 断言 `failures: 7`）。
- **C1-S2 阈值来源**：`consecutive_failures >= MAX_CONSECUTIVE_GOAL_ROUND_FAILURES`（`=2`）。
  阈值只允许该常量表达，禁止散落字面量 `2`（红相 `:117-123` 锁常量值）。
- **C1-S3 `visible_line()` 逐字冻结（wave-68-a3：由六条扩为**七条**，覆盖枚举全部变体；红相只锁
  `已停止`/原因词/不含 `进行中`/含轮次用量，实现必须逐字取值以消除歧义）**：
  | 变体 | `visible_line()` | 变体性质 |
  |---|---|---|
  | `Cancelled` | `goal 续轮已停止（用户取消）` | 冻结（红相） |
  | `ConsecutiveFailures { failures }` | `format!("goal 续轮已停止（连续 {failures} 轮失败）")` | 冻结（红相） |
  | `BudgetExhausted { rounds_started, max_rounds }` | `format!("goal 续轮已停止（目标轮次预算已用尽：{rounds_started}/{max_rounds}）")` | 冻结（红相） |
  | `AutoContinueBudget` | `goal 续轮已停止（自动续轮次数用尽）` | additive（C1-S4） |
  | `TurnSuspended` | `goal 续轮已停止（本轮挂起：等待用户作答/审批）` | additive（C1-S5） |
  | `TurnUnclassified` | `goal 续轮已停止（本轮无可用终态，已暂停自动续轮）` | additive（C1-S5） |
  | `LoopAlreadyOwned` | `goal 续轮已停止（本会话已有续轮循环在运行，本线程让位）` | additive（C1-T2 租约未抢到） |
  实现取证（wave-68-a3 实测）：`crates/pony-agent-core/src/agent/goal.rs:453-473`（`visible_line` 全文）、
  `:445`（`LoopAlreadyOwned` 变体定义）。
- **C1-S4（additive 变体）**：为保持 F1 的「8 轮自动续轮预算」（`should_abort_goal_rounds`）**既有语义
  不变**，`GoalRoundStopReason` 追加第四个变体 `AutoContinueBudget`。该变体**不由**
  `goal_round_stop_reason` 产出（其入参不含自动续轮计数），由 TUI 侧在
  `should_abort_goal_rounds(cancelled, consecutive, MAX_CONSECUTIVE_AUTO_CONTINUES)` 命中时构造。
  追加变体不破坏红相（其 `matches!` 只命中具体变体）；另两个 additive 变体见 C1-S5。
- **C1-S5（additive 变体 + 触发条件，wave-68-a2 补登）**：`TurnSuspended` / `TurnUnclassified`
  同样**不由** `goal_round_stop_reason` 产出（§C1-S1 的三值真值表**不变**），而由宿主按轮次结果映射：
  | 轮次结果（`start_turn_stream_checked`） | 宿主动作 | 停止原因变体 |
  |---|---|---|
  | `Ok(Completed)` | 推进一轮预算后继续 | 无（不停止） |
  | `Ok(Failed { .. })` | `failures += 1`，**不推进预算**（C2） | 达阈值时由 `goal_round_stop_reason` 产出 `ConsecutiveFailures` |
  | `Ok(Cancelled)` | 立即停止 | **`Cancelled`（映射 `Ok(Cancelled) = Cancelled`）** |
  | `Ok(Suspended)` | 立即停止（模型挂起等用户） | `TurnSuspended` |
  | `Ok(Unknown)` | 立即停止（不知成没成） | `TurnUnclassified` |
  | `Err(_)`（收口失败 / 无终态） | 立即停止：不重跑、**不计失败、不烧预算** | `TurnUnclassified` |
  为什么必须区分两变体（`[DERIVED]`）：`Suspended` 若被当成功 ⇒「模型挂起等用户时仍多开一轮」，
  正是 ADR 0092 的 16 轮自转面；`Unknown`/`Err` 若被当失败 ⇒ 伪造失败轮并误触 C1 止损阈值。
  证据：`crates/pony-agent-core/src/bin/tui.rs:8230-8305`（主循环分支）、`:8374-8385`（outcome→reason
  映射函数）、`goal.rs:421-443`（变体定义）、`:463-466`（两条文案）。
- **C2-S1 `advance_goal_round_for_outcome` 语义（冻结）**：
  - `Succeeded` ⇒ 复用既有 `advance_goal_round(goal_id)` 语义（`rounds_started +1` + 原子持久化 +
    失败回滚），**不得改 CAS/持久化**（G3）；返回更新后快照。
  - `Failed` ⇒ **不写盘、不 +1**，返回**当前快照**（`Ok`，不是 `Err`）；红相 `:266-283` 连查 3 次
    `rounds_started == 0`，故实现不得以任何形式缓存/延迟递增。
  - goal 不存在 ⇒ `Err`（与 `advance_goal_round` 一致，`goal.rs:370-374`）。
  - 实现必须让 `advance_goal_round` **保持存在且语义不变**（其它调用点仍用：
    `control_plane/mod.rs:1882`、`:2635`、`planner.rs:1547`）。
- **C2-S2 调用时序（冻结，t-w1 唯一允许的时序）**：
  1. 每轮**起轮前**用只读快照 `active_goal_for_session(session_id)` 取 `rounds_started / max_rounds`；
  2. 起轮（`start_turn_stream_checked`，见 §3.1）；
  3. 轮次结束**后**才调 `advance_goal_round_for_outcome(id, outcome)`；
  4. 本轮显示轮号 = `rounds_started + 1`（只读派生，不落库）。
  ⇒ 失败轮与未起轮都不消耗预算；「先推进后起轮」写法（F2）**禁止**。
- **C2-S3 预算耗尽可见性**：`BudgetExhausted` 命中时 TUI 必须把 `visible_line()` 写入状态行
  （`state.status_text`，`tui.rs:1312`）并落一条 System 消息，禁止回落「进行中」
  （历史缺陷 `tui.rs:3827-3846` 的横幅 phase 来自 `GoalData.phase`，本卡不改该映射）。

### 2.3 TUI 接线契约（t-w1，`crates/pony-agent-core/src/bin/tui.rs`）

- **C1-T1 首轮门**：轮次线程的首轮改用 §3.1 的 `start_turn_stream_checked`（`tui.rs:9779` 由
  `start_turn_stream` 换为 checked）；仅 `Completed` 才进入 `run_goal_rounds`
  （`tui.rs:9783-9789`）。`Failed / Cancelled / Suspended / Unknown` 一律不续轮
  （`[DERIVED]`：这是把「一次 429 收敛为一次可见失败」的唯一可靠层；否决该项时 C1 的
  循环内连续失败止损（§2.2 C1-S2）仍满足红相，不影响验收 3）。
- **C1-T2 单循环所有权（wave-68-a3：按实现真值改写）**：`run_goal_rounds` 入口以
  **`type GoalRoundLoopLeaseRegistry = Arc<std::sync::Mutex<std::collections::HashSet<String>>>`**
  按 **`session_id` 分区**抢租约（`tui.rs:8331`）：`try_acquire` 在锁内 `insert(session_id)`
  （`tui.rs:8339-8345`）；返回 `None`（该会话已有循环）时**不得静默 return**——必须以
  `GoalRoundStopReason::LoopAlreadyOwned` 走 `report_goal_rounds_stopped` **可见上报**后返回
  （`tui.rs:8164-8179`，报告点 `:8170-8177`），并落 `warn` 结构化日志
  `goal_rounds_loop_already_owned`（`tui.rs:8352-8357`）。租约由 RAII `Drop`
  `remove(&session_id)` 释放（`tui.rs:8362-8369`，覆盖正常返回 / panic 展开 / 提前 return 三条路径）。
  按会话分区的理由（`[DERIVED]`，与 NFR `concurrency_lock` 及 ADR 0103 同源）：同会话只允许一个
  续轮循环（消除 F14 的 2×8 叠加），而切换会话后另一会话的续轮不被本会话卡住。
  **既有调用点**：registry 在 `tui_event_loop` 内创建（与 `goal_round_cancel` 同级），唯一传入点
  `tui.rs:8164`。**测试锚现状（如实登记，不伪称已有）**：该租约当前**零机械测试锚**——
  `crates/pony-agent-core/tests/` 全目录 grep `GoalRoundLoopLease` / `try_acquire` /
  `LoopAlreadyOwned` / `loop_already_owned` 均 **0 命中**（wave-68-a3 实测），
  属**遗留缺口**、只能靠代码评审覆盖（本卡不新增测试，后续波次可补直接单测）。
- **C1-T3 循环体（冻结伪代码）**：
  ```rust,ignore
  let mut consecutive: u32 = 0;        // 既有 8 轮自动续轮预算（语义不变）
  let mut failures: u32 = 0;           // C1 连续失败计数（线程局部，随循环消亡）
  loop {
      let goal = active_goal_for_session(session_id) else break;
      let pending = session_has_pending_host_requests(turn_control_plane, session_id);
      let reason = goal_round_stop_reason(cancel.is_cancelled(), failures,
                                          goal.rounds_started, goal.max_goal_rounds)
          .or_else(|| should_abort_goal_rounds(cancel.is_cancelled(), consecutive,
                      MAX_CONSECUTIVE_AUTO_CONTINUES as u32)
                      .then_some(GoalRoundStopReason::AutoContinueBudget))
          .or_else(|| pending.then(/* 既有让位：仍直接 break，不新增变体 */ None).flatten());
      if let Some(reason) = reason {
          let line = reason.visible_line();                    // 不静默 break（G7）
          let _ = sink.tx.send(UiEvent::GoalRoundsStopped { reason, status_line: line });
          break;
      }
      // 起轮：round = goal.rounds_started + 1（C2-S2）
      consecutive += 1;
      match turn_control_plane.start_turn_stream_checked(sink.as_ref(), next_cmd) {
          Ok(TurnStreamOutcome::Completed) => {
              failures = 0;
              if advance_goal_round_for_outcome(&goal.id, GoalRoundOutcome::Succeeded).is_err() {
                  let _ = sink.tx.send(UiEvent::GoalRoundsStopped { .. }); // 预算推进失败 ⇒ 停
                  break;
              }
          }
          Ok(TurnStreamOutcome::Failed { .. }) => failures += 1,   // 不推进预算（C2）
          Ok(TurnStreamOutcome::Cancelled)
          | Ok(TurnStreamOutcome::Suspended)
          | Ok(TurnStreamOutcome::Unknown) => break,
          Err(_) => break,                                          // 不推进预算（C2）
      }
  }
  ```
  说明：`pending` 分支保持既有「让位即 break」语义（`tui.rs:8140`），状态行仍由既有
  Ask/审批面板承担，本卡不新增变体（避免过度约束实现）。
- **C1-T3′（wave-68-a2 补登）每个 `break` 都必须带可见原因**：上述循环体内**全部**非成功出口
  （`Ok(Cancelled)` / `Ok(Suspended)` / `Ok(Unknown)` / `Err(_)`）都不得裸 `break`，必须经
  `GoalRoundStopReason`（§C1-S5 映射）→ `report_goal_rounds_stopped` → 结构化日志
  `goal_rounds_stopped` + `UiEvent::GoalRoundsStopped`。实现取证：
  `crates/pony-agent-core/src/bin/tui.rs:8230-8305`、`:8341-8361`、`:8395-8414`。
- **C1-T4 事件与状态字段（additive）**：
  - `UiEvent::GoalRoundsStopped { reason: GoalRoundStopReason, status_line: String }`
    （加入 `UiEvent`，`tui.rs:198-292`；事件循环 arm 置 `state.status_text = status_line.clone()`
    并 push 一条 `MessageRole::System` 消息）。
  - `TuiState` 增 `pub goal_rounds_stopped: Option<GoalRoundStopReason>`（`tui.rs:1244-1320`）；
    复位点与 `goal_round_cancel.reset()` 同处（`tui.rs:9736`）。
  - 严重级/配色沿用既有 `system_message_severity`（`errors.rs:95-119`），**本卡不新增颜色规则**。

---

## 3. C1（core 半）与 C3：轮次 outcome + 限流分类/冷却

### 3.1 宿主流式轮 outcome（冻结面，写域 `control_plane/mod.rs` = `t-w1`）

```rust,ignore
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamOutcome {
    Completed,
    Failed { error: Option<String> },
    Cancelled,
    Suspended,
    Unknown,
}

impl HostControlPlane {
    /// 新增：与 `start_turn_stream` 同语义，但把轮次终态回传宿主。
    pub fn start_turn_stream_checked<S: TurnEventSink>(
        &self,
        sink: &S,
        command: StartTurnStreamCommand,
    ) -> Result<TurnStreamOutcome, String>;

    /// 既有签名**冻结不变**（`mod.rs:2139-2146`），内部转调 checked 并丢弃 outcome。
    pub fn start_turn_stream<S: TurnEventSink>(&self, sink: &S, command: StartTurnStreamCommand);
}
```

- **C1-A1 outcome 派生规则（冻结）**：`start_turn_stream_checked` 以
  `RecordingTurnEventSink`（`control_plane/mod.rs:750-761`）包住宿主 sink 后调用
  `start_ask_aware_turn_stream`（`ask_resume_commands.rs:31-118`），再按
  **`GraphRunTurnResponse.decision.kind`（Fail/Cancel/WaitUser）+ turn phase** 派生：
  | 终态/决策 | outcome |
  |---|---|
  | phase `completed` 且 decision 非 Fail/Cancel | `Completed` |
  | phase `failed`（`turn_flow.rs:573`）或 decision `Fail` | `Failed { error }`（error 取终态事件的 `error`，见 C1-A2） |
  | phase `cancelled`（`turn_flow.rs:639/760`）或 decision `Cancel` | `Cancelled` |
  | phase `suspended`（`runtime/types.rs:313`）或 decision `WaitUser` | `Suspended` |
  | 无终态可采（`build_turn_result()` 返回 `None`） | `Unknown`（**不得**当成功） |
  `Err(String)` 仅在既有失败面透传：`execute_graph_run_stream_step` 失败
  （`ask_resume_commands.rs:85-103`）或 run 解析/执行失败；此时**不得**额外补发终态事件
  （避免双终态），并必须视为「本轮未成功」。
- **C1-A2 error 采集（t-w1 写域内，不改其它文件）**：`RecordedTurnTerminal`
  （`control_plane/mod.rs:718-748`）追加 `error: Option<String>`，在 `record_terminal_payload`
  （`:820-852`）从 `payload.error` 采集。理由：`TurnResult` 无 error 字段（`runtime/types.rs:106-136`），
  故不回写 `TurnResult`、不影响 graph 语义。
- **C1-A3 legacy 兜底路径**：`start_ask_aware_turn_stream` 回落
  `start_turn_stream_legacy`（`ask_resume_commands.rs:104`）时，外层 `RecordingTurnEventSink`
  仍能采到终态（F5），故 legacy 路径同样产出真实 outcome；**禁止**因此修改
  `ask_resume_commands.rs`（不在 t-w1 写域）。
- **C1-A4（wave-68-a1 补登）outcome 派生判别式（封闭，冻结）**：`start_turn_stream_checked`
  必须按如下**单一优先序**判别，不得引入第二种判据：

  | 序 | 判据（按序命中即返回） | outcome |
  |---|---|---|
  | 1 | `GraphRunTurnResponse.decision.kind == Fail` | `Failed { error }` |
  | 2 | `decision.kind == Cancel` | `Cancelled` |
  | 3 | `decision.kind == WaitUser` | `Suspended` |
  | 4 | 其余决策 ⇒ 按 turn phase：`failed` / `cancelled` / `suspended` / `completed` | 依次 `Failed { error }` / `Cancelled` / `Suspended` / `Completed` |
  | 5 | 无终态可采（`build_turn_result()` 返回 `None`，或 phase 不在上述四值） | `Unknown`（**不得**当成功） |

  即：**`Fail⇒Failed` / `Cancel⇒Cancelled` / `WaitUser⇒Suspended` / 其余按 phase**。决策序优先于
  phase 的理由（`[DERIVED]`）：`decision.kind` 是 graph 层对轮次结果的显式裁决
  （`control_plane/mod.rs:2025-2035`，含 `graph.rs:829-836` 的 Cancel 归一），phase 只是终态事件
  的文本相位（`turn_flow.rs:573`），二者冲突时以显式裁决为准。
  本项同时**闭合 C1-A2 的落地条件**：序 1/4 的 `Failed { error }` 需要的 `error` 正是由 C1-A2 在
  `RecordedTurnTerminal` 新增的 `error: Option<String>` 采集（否则 `Failed { error: None }` 恒空，
  状态行与审计拿不到失败文本）。`Err(String)` 通道的语义不变（见上「既有失败面透传」）。

### 3.2 限流分类（冻结面，写域 `agent/retry.rs` = `t-w2`）

```rust,ignore
/// 失败归类唯一出口（C3 冷却判定 / C4 审计字段 / C5 文案共用，禁止第二套）。
pub fn classify_error_class(err: &str) -> &'static str;      // timeout|rate_limited|quota|network|auth|bad_request|other
/// 是否属于「应冷却 provider」的类别（rate_limited | quota）。
pub fn is_rate_limit_cooldown_error(err: &str) -> bool;
/// 从错误文本解析可读恢复时间（`retry_after=<n>` / `resets at HH:MM` 等）；解析不到 ⇒ None。
pub fn rate_limit_recovery_hint(err: &str) -> Option<String>;
/// 恢复窗口毫秒数；`now_utc_hm` = 当前 UTC 时刻（时分），供「resets at HH:MM」求差。
pub fn rate_limit_window_ms(err: &str, now_utc_hm: (u8, u8)) -> Option<u64>;
```

- **C3-S1 判定优先级（冻结；同时满足 C3 红相与 C4 真值表）**：
  `quota` → `rate_limited` → `timeout` → `auth` → `network` → `bad_request` → `other`。
  - quota 词（复用 `RATE_LIMIT_LONG_BACKOFF_HINTS` 内 `quota` + 新增 `usage limit` / `allowance` /
    `billing window`）：断言见红相 C4 truth table 第 3 条（`retry.rs:344-352` 常量扩展点）。
  - `rate_limited`：`429` / `rate limit` / `rate_limit` / `ratelimit` / `too many requests` / `tpm`。
  - `timeout`：`UPSTREAM_TIMEOUT_HINTS`（`retry.rs:306-311`）；`auth`：`UPSTREAM_AUTH_HINTS` 与
    `401/403/api key/invalid_api_key`；`network`：`UPSTREAM_NETWORK_HINTS` + `UPSTREAM_TRANSPORT_HINTS`；
    `bad_request`：`UPSTREAM_BAD_REQUEST_HINTS` 的其余码 + `invalid request` / `missing field`。
  - 判定必须复用 `strip_endpoint_fragments` + `hints_match`（`retry.rs:357-380`、`:531-552`），
    **不得**退回裸 `contains`（端口数字污染，PA-143/PA-144）。
- **C3-S2 `is_rate_limit_error` 语义冻结、码表**只允许**扩展**：追加
  `"usage limit"`、`"allowance"`（红相 `:44-56` 的第 ⑤⑥ 条 `RATE_LIMIT_TEXTS`），
  既有七词与其判据不变（`retry.rs:344-352`、`:746-752`）。
  `[DERIVED]` 副作用登记：扩表后 `Followup` 作用域的 rate-limit 长退避
  （`provider/mod.rs:4677-4680`）也会覆盖配额类文本；这是既有 quota 词的同一后果，不属本卡改动。
- **C3-S3 `upstream_code_for_hints` 对 quota-only 词返回既有 `ErrorCode::UpstreamQuota`**
  （F10）：新增分支必须放在 429 组判定**之后**、5xx/其余判定**之前**，且**不得**改变
  `ProviderRetryPolicy::classify` 对 quota 的既有归类结论（G3：`classify` 把 `UpstreamQuota`
  落 `NonRetryable`，与今天 quota-only 的 Abort 一致 ⇒ 行为不变）。
- **C3-S4 `Retry-After` 上线（唯一的错误文本结构化后缀，两段解析同源）**：
  ```rust,ignore
  pub const RETRY_AFTER_MARKER: &str = "retry_after=";
  pub fn append_retry_after_marker(err: &str, seconds: u64) -> String;
  pub fn parse_retry_after_marker(err: &str) -> Option<u64>;
  ```
  冻结理由（`[DERIVED]`）：`Result<T, String>` 是 6 个重试调用点的既有管道，改签名会波及
  `t-w2` 写域之外；而 C5 的可见文案与 C4 的审计都需要同一份恢复时间，故以**唯一解析点**
  承载，禁止第二处拼装。

### 3.3 冷却（冻结面，写域 `agent/provider/mod.rs` = `t-w2`）

```rust,ignore
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCooldownEntry {
    pub until_unix_ms: u64,
    pub class: &'static str,          // classify_error_class 的返回值
    pub retry_after_s: Option<u64>,
}

/// 进程级注册表（ProviderManager 是每轮新建的只含 config 的结构，见 `provider/mod.rs:612-619`）。
pub struct ProviderCooldownRegistry { /* Mutex<HashMap<String, ProviderCooldownEntry>> */ }
impl ProviderCooldownRegistry {
    pub fn global() -> &'static ProviderCooldownRegistry;                    // OnceLock
    pub fn remaining(&self, key: &str) -> Option<Duration>;                  // 认 now=Instant::now()
    pub fn enter(&self, key: &str, entry: ProviderCooldownEntry);
    pub fn clear(&self, key: &str);
}

impl ProviderManager {
    pub fn cooldown_remaining(&self) -> Option<Duration>;   // 宿主/测试可观测
    pub fn clear_cooldown(&self);                           // 恢复开关/测试复位
    /// 冷却键：**四段** `requested_name|provider_name|model|base_url`（D68-7，见 C3-P7）。
    fn cooldown_key(&self) -> String;
    fn ensure_not_cooling(&self) -> Result<(), String>;     // 冷却期直接 Err（0 次 HTTP）
    fn enter_cooldown(&self, error: &str);                  // 仅 rate_limited|quota 类
}
```

- **C3-P7（wave-68-a1 补登）冷却键 = 四段，含 `base_url`（D68-7，Lead 已批准）**：
  `provider_cooldown_key = "{requested_name}|{provider_name}|{model}|{base_url}"`
  （实现取证：`crates/pony-agent-core/src/agent/provider/mod.rs:721-729`，四段字面量逐段取自
  `self.config`）。
  **理由（D68-7）**：同一进程内不同 endpoint（含本地回环 mock / 不同网关环境）各自拥有**独立的配额
  窗口**；若只用三段键，同一 provider/model 在一个 endpoint 上撞 429 后，另一 endpoint（以及 C3-d 的
  503 反向钉子用例）会被误判为「冷却中」直接短路 ⇒ 生产上造成**跨环境误杀**（一个环境的限流冻结
  掉另一个环境的正常调用），且会让反向钉子用例变成假绿。
  纪律：`base_url` 参与键 ⇒ 冷却语义是「**每个 endpoint 一份窗口**」，不是「每个 provider 一份」；
  任何新增键段必须走同一函数，禁止在调用点另行拼键。

- **C3-P1 接入点（冻结）**：`ensure_not_cooling()?` 加在 F8 的 **4 个 HTTP 出口发送前**
  （`provider/mod.rs:2126`、`:2179`、`:2524`、`:2565` 的 `let request = ...` 之前）；
  非 2xx 分支改为调用唯一装配点 `fn provider_status_error(status, elapsed_ms, body, retry_after: Option<&str>) -> String`
  并在返回 `Err` 前 `self.enter_cooldown(&error)`（仅限限流/配额类）。
  `Retry-After` 必须在 `response.text()` **之前**读取（`reqwest::Response::text(self)` 消费响应，
  `:2533-2540`）。
- **C3-P2 快速失败**：`retry_provider_scoped_with_report`（`provider/mod.rs:4595-4612`）在
  `policy.classify(&err)` 之后（`:4676`）、`use_rate_limit` 块之前（`:4677`）插入：
  `if scope == ProviderRetryScope::Decision && is_rate_limit_cooldown_error(&err) { return (Err(last_error), report(...)); }`
  即 Decision 作用域**首次失败即 Abort**（0 次退避 sleep）。**双重机制必要性**：仅靠出口 gate
  时重试循环仍会 sleep 0.5+1+2+4 = **7.5s**，违反红相 `elapsed < 3s`（`pa171_provider_rate_limit_cooldown_contract.rs:246-260`）。
- **C3-P3 冷却错误文案（冻结）**：
  `format!("provider {label} 处于限流冷却中（{class}），预计 {secs}s 后恢复；本次未发起请求；retry_after={secs}")`
  ——必须含 `冷却`/`恢复`/`retry_after=` 任一可读恢复信息（红相 `mentions_recovery_window`，`:199-208`）。
- **C3-P4 线程安全要求（NFR 对齐，见 `nfr-baseline-wave68.json`）**：`OnceLock<ProviderCooldownRegistry>` +
  内部 `Mutex<HashMap<..>>`；锁只用于 get/insert/evict，**禁止**持锁跨 HTTP/`block_on`/sleep；
  poison 恢复沿用既有写法 `lock().unwrap_or_else(|e| e.into_inner())`（`control_plane/mod.rs:2168-2171`）；
  条目容量硬上限 + 过期即淘汰（`resource_release` 登记）；冷却**不新增线程/定时器**（G4）。
- **C3-P5 冷却窗口上限**（`[DERIVED]`）：`retry_after` 缺失时取 60s；有值时按值钳到 6h 上界
  （Kimi 5 小时窗口足以覆盖），防止异常头把 provider 永久冻结。
- **C3-P6 非限流类必须保留既有重试**（红相反向钉子 `:318-343`：503 ⇒ ≥2 次 HTTP）：
  `timeout`/`server_error`/`network` 一律不冷却、不早退。

---

## 4. C4：失败轮归因（红相：`tests/pa171_failed_turn_attribution_contract.rs`；写域 `t-w3`）

### 4.1 冻结面（**逐字**抄录语义 + Lead 指定的 additive 面）

```rust,ignore
// 1) 既有 14 参签名**冻结**（turn_flow.rs:532-549；红相 :68-77 按此顺序实参调用）
pub fn emit_stream_failed(
    sink: &impl TurnEventSink,
    turn_id: String,
    provider_meta: Option<&ProviderEventMeta>,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    build_context_observation: Option<BuildContextObservation>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    hook_trace_records: Option<Vec<HookTraceRecord>>,
    error: String,
    session_id: Option<String>,
    partial_text: Option<String>,
) -> TurnEventEnvelope;                        // 内部转调 attributed 旁路（不新增形参）

// 2) 新增归因载体 + 旁路入口
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FailedTurnAttribution {
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub error_class: Option<String>,
}
impl FailedTurnAttribution {
    pub fn from_failure(provider_meta: Option<&ProviderEventMeta>, error: &str) -> Self;
}
pub fn emit_stream_failed_attributed(/* 上述 14 参原序 */, attribution: FailedTurnAttribution)
    -> TurnEventEnvelope;

// 3) wire additive 字段
//    TurnStreamEvent.error_class: Option<String>
//    #[serde(default, skip_serializing_if = "Option::is_none")]  ⇒ 序列化键 errorClass
```

### 4.2 语义约束（补齐）

- **C4-S1 三字段真值（冻结；红相 `:122-158`）**：
  | 字段 | `provider_meta = Some` | `provider_meta = None` | 依据 |
  |---|---|---|---|
  | `provider_source` | `format!("provider_attempt_failed:{}", meta.requested_name)`（必须**含真实 provider 名**，如 `magpie`/`kimi`） | `None` | 红相要求含 `magpie`/`kimi`；`:148-155` |
  | `provider_mode` | `"failed"` | `None` | 与失败 `TurnResult` 同值（F11 `turn_flow.rs:456`），且非占位词 |
  | `fallback_reason` | 取 **§C4-S5 的封闭值域**（`provider_decision_failed` / `provider_followup_failed` / `turn_failed_no_fallback`） | `None` | 触发方标签优先；未命中任何触发方标签 ⇒ 默认值（详见 C4-S5） |
  | `error_class` | `Some(classify_error_class(&error))` | 同（**永远有值**） | 红相 `:203-216` 要求 wire 上键存在 |
  红相 `assert_non_placeholder` 禁止 `""`/`none`/`null`/`unknown`（`:104-116`），上表逐项满足。
- **C4-S2 失败语义不得被归因破坏**（G6）：事件名仍 `turn:failed`、`kind == "failed"`、
  `event.error` 原样等于入参 `error`、`text` 仍走 `build_failed_turn_text`
  （`turn_flow.rs:507-513`、`:576`）；`capture_turn_failure` 上报保留（`:552-556`）。
- **C4-S3 落盘同源（t-w3 写域内）**：失败路径的 checkpoint 与 trace record 必须传同一组真值：
  `turn_persist.rs:461-475`（`update_execution_checkpoint` 的 provider_source/provider_mode/fallback_reason）
  与 `:476-502`（`persist_turn_trace_with_provider_calls_and_hooks` 的 `provider_source`/`provider_mode`/
  `fallback_reason` 三个形参）不得再传 `None`；record 字段落点 `:805-806`、`:810`。
  `error_class` 同时写入 `TurnTraceRecord`（`session/types.rs:709-787`，additive
  `#[serde(default, skip_serializing_if = "Option::is_none")]`，键 `errorClass`）。
- **C4-S4 单一分类器**（G1）：`error_class` 值只能来自 `retry::classify_error_class`（§3.2），
  禁止在 `turn_flow.rs` 内写第二套映射。
- **C4-S5（wave-68-a1 补登 / wave-68-a2 定级）`fallback_reason` 封闭值域 + 判定优先级 + 例外**：
  L3-P 指出原矩阵只登记了默认值、与实现（按**触发方标签**落值）不一致。此处把实现取值**全部登记**为
  **封闭值域**（3 个字面量 + 1 个缺席态），任何第 4 个字面量都属契约违规。
  **⚠ 字段性质（wave-68-a2 明确，防误标）**：这是**既有字段的语义变更**（`fallback_reason` 早已存在于
  `TurnStreamEvent` / `TurnTraceRecord` / checkpoint，字段名/类型/serde 形状都不变），取值由
  `None`（历史硬编码，`turn_flow.rs:583`）→ 三个字面量 —— **向后兼容但非 additive**：
  旧消费者不会报错，但**读到的值会变**（`null` → 语义字符串）。故本卡对 C4 **不得**沿用
  「additive only」表述（G2 仅适用于 `error_class` 这个真正新增的键）；
  审计机若按「`fallbackReason == null` ⇒ 未发生降级」的旧口径判断，必须同步改为按本次封闭值域判读。

  | # | 取值（逐字） | 触发方 | 判定优先级 | 证据 |
  |---|---|---|---|---|
  | 1 | `provider_decision_failed` | **首跳 decision** 的模型调用失败（provider 类失败） | 最高（先于 2/3） | `runtime/turn_stream.rs:2928-2931`（`fallback_reason: Some("provider_decision_failed")`） |
  | 2 | `provider_followup_failed` | **工具 followup** 的模型调用失败（provider 类失败） | 次之 | `runtime/turn_stream.rs:1226-1246`（触发方标签 `"provider_followup_failed"` 传入失败收口） |
  | 3 | `turn_failed_no_fallback` | **默认值**：没有任何 provider 触发方标签时的失败（工具/钩子/**附件保存失败**等） | 低于 1/2，仅作兜底 | `agent/turn_flow.rs:552-564`（`provider_meta.map(\|_\| "turn_failed_no_fallback")`） |
  | 4 | 键缺席（`None`） | `provider_meta == None`（本轮从未选出 provider） | — | 同上（`provider_meta.map(..)` 在 `None` 时不置值） |

  **判定优先级（冻结）**：`触发方标签 1/2` → `默认值 3` → （`provider_meta == None` 时）`缺席 4`。
  即「有标签用标签、无标签用默认值、无 provider 则缺席」，**禁止**用默认值掩盖已识别的触发方
  （否则审计机无法区分「首跳失败」与「工具 followup 失败」）。
  **优先级真值表 · 示例（wave-68-a2 补登，逐场景冻结）**：
  | 例 | 场景 | `provider_meta` | 触发方标签 | `fallbackReason` 落值 |
  |---|---|---|---|---|
  | E1 | **首跳 decision** 遇 429 配额耗尽（诊断会话 10:28:51 同形态） | `Some(magpie/kimi)` | `provider_decision_failed` | `provider_decision_failed` |
  | E2 | **工具 followup** 模型调用遇 429/5xx | `Some(magpie/kimi)` | `provider_followup_failed` | `provider_followup_failed` |
  | E3 | 首跳成功、随后**工具执行**失败（非 provider 失败） | `Some(…)` | 无 | `turn_failed_no_fallback`（默认值） |
  | E4 | **附件保存失败**（provider 已选出） | `Some(…)` | 无（例外：不得塞 provider 标签） | `turn_failed_no_fallback`（默认值） |
  | E5 | provider 未选出（缺 key/配置）即失败 | `None` | 无 | 键缺席（`null`） |
  **落地约束**：标签只能由 `runtime/turn_stream.rs` 的上述两个 provider 失败入口写入；
  非 provider 失败**必须**走不带标签的同名入口（`turn_persist.rs:493-495` 注释即为该纪律的原文）。
  **例外（明确登记）**：**附件保存失败**（以及工具/钩子类失败）即使发生在 provider_meta 存在的轮次，
  也**走默认值 `turn_failed_no_fallback`**——因为它们不是 provider 触发方，硬塞标签会伪造归因。
  同名前缀的**非失败轮**用法：`agent/provider/mod.rs:5017` 的
  `format!("provider_followup_failed: {}", preview_text(&error, 240))` 属
  `ProviderDecision.fallback_reason`（本地工具 fallback 的降级原因，`provider_mode = "fallback"`），
  **不是**失败轮事件的值域成员；二者必须共用同一标签来源，禁止各自拼字面量（G1）。

---

## 5. C5：错误文案如实（红相：`tests/pa171_error_copy_contract.rs`；写域 `bin/tui/errors.rs` = `t-w2`）

```rust,ignore
/// C5：限流/配额类终态摘要的固定开头 + 无恢复窗口时的完整文案。
pub const ERROR_SUMMARY_RATE_LIMITED: &str = "上游限流/配额受限";
pub const ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW: &str = "上游限流/配额受限，恢复时间未知，请稍后重试。";

/// C5：限流/配额命中时的可读摘要（含恢复时间）。
pub fn rate_limited_error_summary(error: &str) -> String;

/// 既有签名冻结：内部 = `user_facing_error_summary_at(error, now_unix_ms())`。
pub fn user_facing_error_summary(error: &str) -> String;
pub fn user_facing_error_summary_at(error: &str, now_unix_ms: u64) -> String;
```

- **C5-S1 判定顺序（冻结）**：`auth`（既有 `errors.rs:139-145` 判据）→ **限流/配额**（
  `classify_error_class` ∈ {`rate_limited`,`quota`}）→ `is_transient_provider_error` →
  `ERROR_SUMMARY_GENERIC`。限流分支必须**插在 auth 之后、transient 之前**：否则 429 命中
  `is_transient_provider_error`（`errors.rs:146`）被折叠成 `ERROR_SUMMARY_TRANSIENT`
  （=`上游服务暂时不可用，重试已用尽。`，`errors.rs:76`）——正是 F13 的缺陷（红相 `:66-96`）。
- **C5-S2 文案冻结**：
  - 有恢复窗口：`format!("{ERROR_SUMMARY_RATE_LIMITED}，{}。", rate_limit_recovery_hint(error).unwrap_or_default())`
    （`hint` 形如 `预计 3600s 后恢复` / `预计 06:01 UTC 恢复`）。
  - 无恢复窗口：`ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW`。
  两种形态都必须含 配额/限流/恢复 之一（红相 `mentions_real_cause`，`:55-68`），且不得等于
  `ERROR_SUMMARY_TRANSIENT`。
- **C5-S3 冻结不变**：`ERROR_SUMMARY_TRANSIENT` / `ERROR_SUMMARY_AUTH` / `ERROR_SUMMARY_GENERIC`
  逐字不变（`:76-78`），`is_transient_provider_error` **一行不改**（`:60-84`，红相反向钉子依赖
  auth 与未知错误的既有映射，`:100-160`）。
- **C5-S4 严重级**：新增摘要必须以既有 `system_message_severity` 判为 `Error`
  （`errors.rs:95-119` 的受控文案集合追加两个新常量；动态文本以 `ERROR_SUMMARY_RATE_LIMITED`
  前缀判定），否则限流错误会被渲染成灰色 Info。

---

## 6. C6：观测归属（只读 `docs/observability.yaml`；由 Lead 收口阶段落笔）

| 归属点 | 本轮新增事实 | 落笔位置 |
|---|---|---|
| `dimensions.tracing`（`observability.yaml:14-40`，anchor `agent/otel/mod.rs`） | 失败轮 trace 新增 `errorClass` 与三字段真值 ⇒ 本地取证面增强，**不新增网络遥测面** | `dimensions.tracing` 的归属说明块追加一条备注 + `reviewed_at` 更新 |
| `dimensions.metrics`（`:45-56`，结构性 N-A） | 冷却注册表是**进程内内存**、无监听地址 ⇒ N-A 理由不变（禁止为让巡检变绿而伪造 `/metrics`） | 无需改判定，仅在 note 里登记「冷却状态为进程内状态，不构成 pull 面」 |
| `enforcement_points.error_classifier`（`:78-91`，anchor `runtime/turn_control.rs`） | **不得**把本卡新增分类器塞进该 anchor（其注释明确只覆盖 Ask 不可恢复挂起族）；`classify_error_class` 的唯一家在 `agent/retry.rs` | 在 `note` 追加一句归属声明 + 禁止第二套码表的判据 |
| 门禁 | `audit-diff.sh --base d9425a70` 与 `check-declaration.sh <repo>` 双 Exit 0 | 任务卡 §验收 6 |

> 本矩阵**只读** `docs/observability.yaml`（Protocol 不改它）。文件修改权属 Lead 收口阶段。

### 6.1 `wave68_logging_events` 契约归属（wave-68-a1 补登）

**前提（结构性）**：本项目 metrics 维度是**结构性 N-A**（`docs/observability.yaml:45-56`：桌面/TUI
交付形态无常驻监听地址、无 Service、无 `/metrics`，pull 模型前提不成立；logs 亦为结构性 N-A，
`:57-66`）。因此**冷却与续轮止损这两类"抑制了多少次失败"的事实，唯一可观测面就是结构化 JSON 日志
（带 `trace_id`）**——不作为「新增遥测面」，也不得为让巡检变绿而伪造 `/metrics`。
三个事件名的家与封闭字段清单如下（字段即声明清单，**禁止夹带响应正文/密钥**）：

| # | 事件名（逐字） | 归属契约 | 唯一发射点 | 字段（封闭） | 语义 |
|---|---|---|---|---|---|
| 1 | `provider_cooldown_entered` | C3（§3.3） | `crates/pony-agent-core/src/agent/provider/mod.rs:787-800`（`ProviderManager::enter_cooldown`，`log_json_with_fields` warn 级） | `trace_id`(自动) / `provider_name` / `model` / `error_class` / `retry_after_s` / `until_unix_ms` | 一次 `rate_limited`\|`quota` 失败**首次**把该 provider 置入冷却；审计机据此算「一次 429 被抑制成多少轮」 |
| 2 | `provider_cooldown_short_circuit` | C3（§3.3） | `provider/mod.rs:752-764`（`ensure_not_cooling`，warn 级） | `trace_id`(自动) / `provider_name` / `model` / `remaining_ms` | 冷却窗口内**0 次 HTTP** 的唯一物理证据（历史缺陷是每轮再打 5 次） |
| 3 | `goal_rounds_stopped` | C1/C2（§2.2/§2.3） | `crates/pony-agent-core/src/bin/tui.rs:8395-8414`（`log_goal_rounds_stopped`，info 级；唯一结构化出口，禁止退回裸 `eprintln`——`:8390-8394`） | `trace_id`(自动) / `session_id` / `reason` / `consecutive_failures` / `rounds_started` / `max_rounds` | 续轮停止原因（取代历史静默 `break`）；`reason` 取值为 `goal_round_stop_reason_tag` 的封闭集合（`tui.rs:8377-8387`：`cancelled` / `consecutive_failures` / `budget_exhausted` / `auto_continue_budget` / …） |

**同族但未列入上述三项的日志面（如实登记）**：`goal_rounds_loop_already_owned`
（`tui.rs:8321`，warn；对应 C1-T2 租约未抢到）与 `goal_round_turn_failed`
（`tui.rs:8287-8290`，warn；`Err` 通道无终态）同属本波次新增可观测面，
字段以代码为准，`nfr-baseline-wave68.json` 的 `wave68_logging_events` 已声明前者；
本补登**不新增**事件名，只登记归属与观测面地位。

**门禁口径**：三事件的「已落地」判据是「发射点存在 + 字段清单一致」，由 L3 审查/`audit-diff.sh`
比对；本矩阵不声称任何 metrics/logs 维度由 N-A 转为 applicable。

---

## 7. 写域、跨文件依赖与合并顺序

| 契约 | 文件 | 写域（`.dev-team/state.json`） | 依赖 |
|---|---|---|---|
| C1 core / C2 | `agent/goal.rs`（新增 5 个符号）、`control_plane/mod.rs`（checked 入口 + `RecordedTurnTerminal.error`）、`bin/tui.rs`（循环/事件/状态） | t-w1 ✅ 全覆盖 | `retry` 无关 |
| C3 | `agent/retry.rs`、`agent/provider/mod.rs` | t-w2 ✅ 全覆盖 | 无 |
| C4 | `agent/turn_flow.rs`、`runtime/turn_stream.rs`、`runtime/turn_persist.rs` | t-w3 ✅ 部分 | **依赖 t-w2 的 `classify_error_class`**（合并顺序：t-w2 先于 t-w3，或同一合并批次） |
| C4（wire 字段） | `agent/runtime/types.rs:141-215`（`TurnStreamEvent.error_class`） | ❌ **不在任何写域** | 需 Lead 在 `state.json` 把 `crates/pony-agent-core/src/agent/runtime/types.rs` 增补进 `t-w3.write_scopes`（本矩阵按 Lead 指示已冻结该字段，**不得**因门禁拦截而放弃字段——放弃即红相 C4-c 必红） |
| C4（落盘字段） | `agent/session/types.rs:709-787`（`TurnTraceRecord.error_class`） | ❌ **不在任何写域** | 同上，需一并在 `t-w3.write_scopes` 增补；若 Lead 拒绝，则 `error_class` 仅落 wire（红相仍绿），并在 `.dev-team/` 登记「trace 侧缺席」的偏差 |
| C5 | `bin/tui/errors.rs` | t-w2 ✅ 全覆盖 | 依赖 t-w2 的 `retry::classify_error_class`（同写域，无跨分支） |
| C6 | `docs/observability.yaml` | Lead 收口 | — |

`tests/pa171_*.rs` 归 t-red，**冻结只读**；任何签名调整只能改其调用点，不得改断言。

---

## 8. 偏差声明（如实登记，未伪造任何通过）

1. **空桩纯度门禁不适用于本仓（实测证据，非推断）**：
   `~/.agents/skills/dev-team/scripts/dev-stub-lint.py` 只做 Python AST 校验
   （`ast.parse`，`:76-84`），目录模式只 `rglob("*.py"/"*.ts"/"*.js")`（`:117-118`），
   非 py/ts/js 后缀一律 `errs = []` 空过（`:126-127`）。
   实跑 `python3 …/dev-stub-lint.py crates/pony-agent-core` → **Exit 1**，14 条命中全部落在
   `crates/pony-agent-core/tests/fixtures/plugin/echo-plugin.py`（预置的 MCP 插件**运行夹具**，
   本就不是空桩文件），与 PA-171 无任何因果关系。
   ⇒ 结论：该脚本在本仓**既不能证明 Rust 纯度（对 `.rs` 只能空过），也只会在无关夹具上误报**。
   本矩阵**不声称**它通过，也**不以它作为本波次门禁**。
2. **替代门禁（本波次冻结口径）**：以「本矩阵冻结的签名 + 红相签名一致性」为契约门禁，
   以 `cargo check -p pony-agent-core --all-targets` **Exit 0** 为编译门禁
   （基线 `d9425a70` 实测 Exit 0，见回执 §9）。二者之外不新增自造门禁。
3. **本轮未做的事**：未修改 `src/`、`tests/`、`docs/`；未执行任何 `git add/commit/checkout`；
   未运行红相（红相由 Test Agent 冻结、由 Lead 在波次门禁跑）。
4. **已知未闭环**：U1（`is_streaming` 是否会在某 Err 路径永真）不在本卡范围；
   C3-S2 的 Followup 长退避副作用（扩表带来的覆盖面）按 `[DERIVED]` 登记，不额外改代码。

---

## 9. 收据（本契约的机器凭证）

- `cargo check -p pony-agent-core --all-targets` → **Exit 0**（基线 `d9425a70`）。
- `python3 ~/.agents/skills/dev-team/scripts/dev-nfr-lint.py .dev-team/nfr-baseline-wave68.json`
  → `[PASS] dev-nfr-lint 校验通过: .dev-team/nfr-baseline-wave68.json`，**Exit 0**。
- `python3 ~/.agents/skills/dev-team/scripts/dev-stub-lint.py crates/pony-agent-core`
  → **Exit 1**，14 条命中全在 `tests/fixtures/plugin/echo-plugin.py`（预置插件夹具 + Python-only 工具）；
  见 §8 偏差声明 1：**不构成纯度证据，不作为本波次门禁，也未伪造成"通过"**。

---

### Amendment: wave-69-a1（2026-11-09，PA-171 遗留收口 · 由 PA-172 波次承载）

> 本 amendment 只**追加**登记，不改写 §2–§5 任何冻结断言文字。权威矩阵：`.dev-team/contract-matrix-pa172.md`。

1. **a1-1 §2.2 C1-S3 `visible_line()` 变体表扩容**（additive，**七变体 → 九变体**；实测今日 `goal.rs:421-446` 为 7 个：Cancelled/ConsecutiveFailures/BudgetExhausted/AutoContinueBudget/TurnSuspended/TurnUnclassified/LoopAlreadyOwned）：
   | 变体 | `visible_line()` | 性质 |
   |---|---|---|
   | `BudgetPersistFailed` | `goal 续轮已停止（本轮预算推进落盘失败，已暂停自动续轮）` | wave-69 新增（遗留 #9：不再复用 `BudgetExhausted` 文案） |
   | `NoActiveGoal` | `goal 续轮已停止（当前会话无进行中的目标）` | wave-69 新增（遗留 #20：`tui.rs:8187` 裸 break 补可见原因） |
   两变体**不由** `goal_round_stop_reason` 产出（其三值真值表 C1-S1 不变），由 TUI 侧构造。
2. **a1-2 §6.1 结构化日志事件封闭集扩容**（metrics/logs 仍为结构性 N-A）：
   `provider_failover_switched`、`provider_failover_skipped_cooldown`、`provider_failover_exhausted`、
   `provider_cooldown_reset`、`upstream_policy_clamped`、`failover_candidate_ignored`、
   `goal_rounds_yield_pending_requests`、`goal_round_advance_failed`（字段清单见新矩阵 §4/§8）。
   `goal_rounds_stopped` 的 `reason` 枚举扩为含 `budget_persist_failed` / `no_active_goal`。
3. **a1-3 §3.3 C3-P4 poison 策略显式化（遗留 #3）**：4 处 `into_inner()`（`provider/mod.rs:650/665/674/694`）
   **保持 fail-open 并升格为契约声明**——理由：临界区仅 `HashMap` 内存操作；fail-closed 会让一次历史 panic
   永久冻结该 provider。新增 poison 机械锚点（见新矩阵 §5 R1）。
4. **a1-4 `clear_cooldown`/`cooldown_remaining` 零生产调用点（遗留 #4）→ 已接线**：新增
   `reset_global_cooldowns() -> usize`（`provider/mod.rs`，清空全表 + `provider_cooldown_reset` 日志），
   生产调用点 = **仅显式 `/clear` 命令分支**（`bin/tui.rs` 中 `/clear` 命中后调用 `reset_session_scope()` 的那一处，约 `:9873`）；
   **不得**挂在 `reset_session_scope` 函数体内（该函数另有 5 个会话切换调用点，会清掉别的会话正在生效的进程级冷却）。a1-5 的「复位只能靠进程重启」结论**作废**。
5. **a1-5 冷却键（遗留 #5，⚠ 本条目旧表述已被 D69-12 接替）**：~~`|`→`%7C`、空段→`-`~~ —— 该规则**非单射**（字面 `%7C`/`-` 会碰撞），
   **作废**。现行冻结口径（D69-12）：段内转义按 `%`→`%25` 先、`|`→`%7C` 后、空段→`~empty~`，且 `base_url` 先**去尾斜杠 + scheme/host 小写化**（path 保持原样）。
   键语义仍是「每个 endpoint 一份窗口」（D68-7 不变），统一入口 `cooldown_key_for(&ResolvedProviderSelection)`。
6. **a1-6 §2.3 C1-T2 租约（遗留 #7/#12）**：`GoalRoundLoopLease` 从 `bin/tui.rs` 抽到 lib 模块
   `agent/goal_lease.rs`，类型与语义逐字不变（`Arc<Mutex<HashSet<String>>>` 按 `session_id` 分区、RAII Drop），
   **零机械测试锚**的缺口关闭（新增 `tests/pa171_lease_anchor_contract.rs`）。
7. **a1-7 `start_turn_stream_checked` 五臂归约（遗留 #11）**：`turn_stream_outcome_from_terminal`
   提升为 `pub`，5 决策臂 × 5 phase 臂真值表由集成测试锚定（新矩阵 §7.2）。
8. **a1-8 §2.3 C1-T4 事件（遗留 #13）**：`UiEvent::GoalRoundsStopped` 增 `session_id: Option<String>`；
   非当前会话的停止上报只落日志、不改状态行/消息区。
9. **a1-9 `TuiState.goal_rounds_stopped`（遗留 #10）**：`reset_session_scope` 必须清空，且必须有读取点；
   `turn_loop_in_flight` 由单 bool 改按 `session_id` 分区（遗留 #8）。
10. **a1-10 TS 契约（遗留 #2）**：`errorClass` 进 `docs/contracts/ts-contract.snapshot.json`（38 → 39 键）与
    `src/types/runtime.ts` 两处类型；`scripts/check-ts-contract.mjs` 红绿方向冻结。
