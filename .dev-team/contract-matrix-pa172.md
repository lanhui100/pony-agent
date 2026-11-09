# PA-172 契约矩阵（wave-69）：上游调用时限/重试配置化 + 跨 provider 有界故障转移 + PA-171 遗留收口

> **批准声明（Lead, 2026-11-09）**：本矩阵是 wave-69 的语义真值源。基线提交 **`458b952e`**（`git rev-parse --short HEAD`，
> 实测），所有 `path:line` 以该基线为准。冲突裁决规则：**签名/命名以本矩阵为准；断言语义不得放宽**。
> 冻结测试（`crates/pony-agent-core/tests/pa171_*.rs` 四个文件）**只读**，任何实现选择都不得让其断言变弱。

## 0. 事实基线（已取证，`path:line`）

| 编号 | 结论 | 证据 |
|---|---|---|
| F1 | 45s 同步总超时是**两处内联字面量**，无常量/无 env/无配置通道 | `provider/mod.rs:2737`（`post_openai_json`）、`:2788`（`post_anthropic_json`）；`.timeout(timeout)` 在共享 builder `:3097-3101` |
| F2 | 流式是另一套预算：15s connect + 180s read-idle，**无总 deadline** | `provider/mod.rs:2306-2311`、`:2358-2363`、`:3129-3137` |
| F3 | 重试次数硬编码：`const PROVIDER_TIMEOUT_RETRY_MAX_ATTEMPTS: u32 = 5`，生效为 `max_retries = 5-1` | `provider/mod.rs:4635`、`:4773` |
| F4 | 退避表：500ms 起 ×2、上限 8s、sleep 预算 30_000ms、无抖动；Followup 限流长退避 20s→40s/2 次/90s | `provider/mod.rs:4771-4779`、`:4788-4796` |
| F5 | 现状**无跨 provider 故障转移**；仅有同 provider 内降级 | followup 本地兜底 `provider/mod.rs:5092-5094`、流式→同步重发 `:1279-1317`、`runtime/tool_recovery.rs:471-475`；`RecoveryPipeline`（`recovery.rs:97-101`）生产未接线 |
| F6 | 配置真值源 `agent/config.rs`：存储 `ProviderConfigStorage:191-210`、顶层 `ProviderRegistryStorage:212-215`（`providers[] + selectedProviderId`，**无** timeout/retry/候选）；视图 `ProviderConfigView:165-183`/`ProviderRegistryView:185-188`；选择算法单值三级回落 `:384-400` | `agent/config.rs` |
| F7 | 配置 round-trip 面：`storage_from_view:520`、`build_view_from_storage:607`、`sanitized_storage_for_persistence:687`、`normalize_storage:847`；前端 `load/save_provider_registry`（`src-tauri/src/lib.rs:595/600/605`）↔ `src/types/provider.ts:63-78` | 同左 |
| F8 | `ProviderSelectionResolver` trait 在 `config.rs:268-274`；实现 13 处，生产 4 处（`config.rs:510`、`bin/tui.rs:2884`、`bin/cli.rs:306`、`subagent_runtime.rs:39`） | 同左 |
| F9 | `ProviderManager` 仅一个私有字段 `config`、**无 Clone**、全仓无结构体字面量（只走 `new`）；生产构造 2 处 | `provider/mod.rs:706-712`、`runtime/tool_exec.rs:17`、`runtime/compaction_ops.rs:103` |
| F10 | 承载模型调用的 4 个方法：`decide_with_tools:897`(sync,retry@925)、`decide_with_tools_stream_bounded:974`(stream,retry@1010)、`continue_with_tool_result:1050`(sync,retry@1081/1105；Anthropic `:1126` 裸调)、`continue_with_tool_result_stream_bounded:1169`(stream,retry@1232) | `provider/mod.rs` |
| F11 | 流式已发增量即 Abort 的既有语义 | `provider/mod.rs:1013-1020`、`:1234-1242` |
| F12 | OpenAI followup 分支会把错误吞成 `Ok(local_fallback_response)`，外层看不到 `Err` | `provider/mod.rs:1092`、`:1117` |
| F13 | 冷却注册表在 `provider/mod.rs`（**不在 retry.rs**）：类型 `:633-635`、单例 `:639-645`、`into_inner()` 共 4 处 `:650/:665/:674/:694`、键 `:721-729`、`cooldown_remaining:732`、`clear_cooldown:737`、`ensure_not_cooling:742`、`enter_cooldown:774`；4 出口 gate `:2306/:2358/:2736/:2787`，4 处登记 `:2343/:2406/:2769/:2830` | 同左 |
| F14 | `clear_cooldown`/`cooldown_remaining` **零生产调用点**（仅测试 `:9009-9010`） | 同上 |
| F15 | `classify_error_class` 返回 6 值、**无 server_error**（5xx 只在 `upstream_code_for_hints` 区分） | `retry.rs:875-901`、`:594-634` |
| F16 | `bin/tui/errors.rs` 的 `now_unix_ms` 是真死参（唯一调用 `:170`，形参在 `:178-179` 被丢弃） | `bin/tui/errors.rs:98-104,170,178-179` |
| F17 | 租约：类型 `bin/tui.rs:8331-8336`（`Arc<Mutex<HashSet<String>>>` 按 session 分区）、doc 首句仍写 `Arc<AtomicBool>`（`:8324`）、`try_acquire:8339-8359`、Drop `:8362-8370`、唯一获取 `:8164`、上报 `:8170-8178`、注册表 `:8474-8475` | `bin/tui.rs` |
| F18 | `start_turn_stream_checked:2226-2247`；归约真身 `turn_stream_outcome_from_terminal:698-718`（决策五臂 `:704-710`、phase 五臂 `:712-718`）、枚举 `:678-689`；`start_turn_stream` 丢 Result `:2204-2211` | `control_plane/mod.rs` |
| F19 | `TuiState.goal_rounds_stopped:1374`（init `:1493`），写点仅 `:10020`/`:10665`、**无读取点**；`reset_session_scope:1606-1659` 未清空 | `bin/tui.rs` |
| F20 | `UiEvent::GoalRoundsStopped:278-281` 仅 `reason`+`status_line`、**无 session_id**；构造 `:8392-8395`；消费 `:10663-10678` | `bin/tui.rs` |
| F21 | 落盘失败复用 `BudgetExhausted` 文案 | `bin/tui.rs:8265-8275`、文案源 `agent/goal.rs:459`；另两处静默吞 `control_plane/mod.rs:1947`、`:2756` |
| F22 | `turn_loop_in_flight` 单 bool `:8479`、guard `:2985-3001`、调用 `:10037/:10044-10046/:10518-10519/:10614-10615`、读取 `:10703` | `bin/tui.rs` |
| F23 | 续轮循环两处 break 无可见原因无日志 | `bin/tui.rs:8187`（无 active goal）、`:8191`（有待作答） |
| F24 | TS 契约门禁：`tests/ts_contract_snapshot.rs` 单测试，快照 `docs/contracts/ts-contract.snapshot.json`；夹具 `:90` 填 `error_class: None` ⇒ 因 `skip_serializing_if` 键不上线缆 ⇒ 快照 38 键**不含 errorClass**；`scripts/check-ts-contract.mjs` 只查「Rust 有 / TS 无」单方向（`:130-136`） | 同左 |
| F25 | 前端 `src/types/runtime.ts`：`TurnStreamEvent:1179`、`TurnTraceRecord:795`、`TraceTimelineEntry:283`；`errorClass` 全仓 0 命中 | 同左 |
| F27 | 流式出口**无总 deadline**（15s connect + 180s read-idle，`provider/mod.rs:2306-2311/2358-2363/3129-3137`），且 `timeout` 可重试 ⇒ 单跳流式最坏 ≈ 180s×5 + 退避 ≈ 907s。**因此流式入口不参与换手**（§3.4）。 | 计划审查 A/BLOCKING |
| F26 | **基线编译环境陷阱（实测）**：直接 `cargo test`（默认 `target/`）会因其它 checkout 共享同一 target dir 而链接到**缺 `LoopAlreadyOwned` 的陈旧 rlib**（`libpony_agent_core-6f4f8d58a831bb58.rlib`，12:31），4 个 test target 假红；改用仓库策略槽位 `--target-dir target-test` 后 `tui_batch2_regression` = **96 passed** | 本波实测（见 §12 收据） |

## 1. 全局不变量（破一条即判未过）

| 编号 | 不变量 | 理由/证据 |
|---|---|---|
| G1 | **单一码表**：失败分类只允许扩展 `retry.rs` 既有常量与 `ErrorCode`；`classify_error_class` 的 **7 值**封闭词表**冻结**（`timeout|rate_limited|quota|auth|network|bad_request|other`；`server_error` 不得加入，且有机器锁 M1：见 §3.2），换手触发类由 `retry::failover_eligible(err)` 复用既有码表产出 | `pa171_failed_turn_attribution_contract.rs:164-198` 锁死词表；`retry.rs:875-901` |
| G2 | **默认零行为变更**：未配置 `upstreamPolicy`/`failoverProviderIds` 时，超时=45s、决策尝试=5、退避预算=30s、候选链为空，逐项与 `458b952e` 一致 | ADR 0104 Decision 1 |
| G3 | **冻结语义**：`ProviderRetryPolicy::classify/decide`、`RetryBudget` 判据、`ERROR_SUMMARY_TRANSIENT` 逐字文案、`is_transient_provider_error`、`emit_stream_failed` 14 参签名、`advance_goal_round` 的 CAS/持久化、`should_abort_goal_rounds` 均不改 | PA-171 矩阵 §1 G3 承接 |
| G4 | **不伪造成功**：换手全链耗尽必须返回最后一次真实错误；任何路径不得把失败改写成成功 | 诊断会话教训 |
| G5 | **不静默**：新增停止/跳过/耗尽路径必须产出可见状态行或结构化日志（二者至少其一，停止轮次两者都要） | PA-171 G7 承接 |
| G6 | **流式安全**：已向 sink 产出任一 delta 后**禁止**换手重发（防重复 token / 重复工具调用） | F11 |
| G7 | **无新增线程/定时器**：换手预算用单调时钟差值判定，不 spawn、不 sleep 之外不新增异步原语 | NFR `resource_release` |

## 2. P1/P2：配置面与钳制（写域 `agent/config.rs`）

### 2.1 冻结签名

```rust,ignore
pub const UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT: u64 = 45_000;
pub const UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT: u32 = 5;
pub const UPSTREAM_RETRY_BUDGET_MS_DEFAULT: u64 = 30_000;
pub const UPSTREAM_REQUEST_TIMEOUT_MS_MIN: u64 = 1_000;
pub const UPSTREAM_REQUEST_TIMEOUT_MS_MAX: u64 = 300_000;
pub const UPSTREAM_DECISION_MAX_ATTEMPTS_MIN: u32 = 1;
pub const UPSTREAM_DECISION_MAX_ATTEMPTS_MAX: u32 = 10;
pub const UPSTREAM_RETRY_BUDGET_MS_MIN: u64 = 1_000;   // 收紧：0 预算与 RetryBudget `>=` 判据冲突（审查 B）
pub const UPSTREAM_RETRY_BUDGET_MS_MAX: u64 = 300_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpstreamPolicy {
    // ⚠ 逐字段容错（审查 B/BLOCKING-1）：`#[serde(default)]` 只补缺字段、接不住**类型错**；
    // 而 providers.json 是整体 from_str（config.rs:498-508），一处类型错会整体回落
    // default_registry() 把用户全部 provider 配置顶替。故三个数值字段各自
    // `#[serde(default, deserialize_with = "...")]`：number | 数字字符串 | null | 其它 ⇒ 回落默认 + warn。
    #[serde(default, deserialize_with = "de_u64_lenient")]
    pub request_timeout_ms: u64,
    #[serde(default, deserialize_with = "de_u32_lenient")]
    pub decision_max_attempts: u32,
    #[serde(default, deserialize_with = "de_u64_lenient")]
    pub retry_budget_ms: u64,
}
// `failoverProviderIds` 同理：`#[serde(default, deserialize_with = "de_vec_string_lenient")]`
// —— 非数组/含非字符串项逐项丢弃 + warn `failover_candidate_ignored`，绝不整体失败。
impl Default for UpstreamPolicy { /* 三个 *_DEFAULT */ }
impl UpstreamPolicy {
    /// 越界钳到边界；返回 (钳制后, 被钳制字段名列表)。空列表 ⇒ 未发生钳制。
    pub fn clamped(self) -> (Self, Vec<&'static str>);
    pub fn is_default(&self) -> bool;
}

pub trait ProviderSelectionResolver {
    fn resolve_provider_selection(&self, provider_id: Option<&str>, model_id: Option<&str>) -> ResolvedProviderSelection;
    /// wave-69 additive（**必须带默认实现** ⇒ 13 个既有实现零改动）
    fn upstream_policy(&self) -> UpstreamPolicy { UpstreamPolicy::default() }
    /// wave-69 additive（默认空链 ⇒ 关闭换手）
    fn resolve_failover_chain(&self, primary: &ResolvedProviderSelection) -> Vec<ResolvedProviderSelection> { Vec::new() }
}
```

### 2.2 wire schema（`providers.json` 顶层，additive）

```json
{
  "upstreamPolicy": { "requestTimeoutMs": 45000, "decisionMaxAttempts": 5, "retryBudgetMs": 30000 },
  "failoverProviderIds": ["kimi", "deepseek"],
  "providers": [ ... ],
  "selectedProviderId": "..."
}
```

### 2.3 钳制真值表（封闭，测试逐行锚定）

| 输入 | 输出 | 是否被钳制 |
|---|---|---|
| `requestTimeoutMs = 0` | `1000` | 是 |
| `requestTimeoutMs = 999` | `1000` | 是 |
| `requestTimeoutMs = 45000` | `45000` | 否 |
| `requestTimeoutMs = 300001` | `300000` | 是 |
| `decisionMaxAttempts = 0` | `1` | 是 |
| `decisionMaxAttempts = 11` | `10` | 是 |
| `decisionMaxAttempts = 5` | `5` | 否 |
| `retryBudgetMs = 400000` | `300000` | 是 |
| `retryBudgetMs = 0` | `1000` | 是 |
| `retryBudgetMs = 999` | `1000` | 是 |
| `retryBudgetMs = 1000` | `1000` | 否 |
| 缺省整个 `upstreamPolicy` | `45000/5/30000` | 否 |
| 段内缺字段（如只有 `requestTimeoutMs`） | 其余取默认 | 否 |

- **C2-a 非致命**：非法值**不得**导致配置加载失败或 provider 面不可用；钳制后仍正常选择 provider。
- **C2-b 可见**：发生钳制时落 `warn` 结构化日志 `upstream_policy_clamped`，字段 `field`（被钳制字段名）、`provided`、`clamped_to`、`bound`（与 §4 事件表逐字同源）。禁止静默。
- **C2-c 候选链卫生**：`failoverProviderIds` 中未知 id、重复 id、解析结果与主 provider 的 `(provider_name, base_url)` 相同的 id 一律**忽略**并 `warn` `failover_candidate_ignored`（字段 `provider_id`、`reason ∈ {unknown, duplicate, self}`）；链长上限 8（超出截断并 `warn`）。
- **C2-d round-trip**：`ProviderRegistryView` 必须携带 `upstreamPolicy` 与 `failoverProviderIds`，且 `storage_from_view → build_view_from_storage` 往返**不丢字段**（前端保存后不得被清空）。

## 3. P3/P4：换手语义（写域 `agent/provider/mod.rs`、`agent/retry.rs`、`agent/config.rs`）

### 3.1 冻结签名

```rust,ignore
// retry.rs
/// 换手触发判据：复用既有码表（G1），**不得**新增第二套关键词表。
pub fn failover_eligible(err: &str) -> bool;
/// 换手耗尽的结构后缀（可解析，供审计/文案；与 retry_after= / cooldown_active 同族）
pub const FAILOVER_EXHAUSTED_MARKER: &str = "failover_exhausted=";
pub fn append_failover_exhausted_marker(err: &str, hops: usize) -> String;
pub fn parse_failover_exhausted_marker(err: &str) -> Option<usize>;
/// 时间敏感的恢复提示（遗留 #6）：以真实 now 计算剩余窗口
pub fn rate_limit_recovery_hint_at(err: &str, now_unix_ms: u64) -> Option<String>;

// provider/mod.rs
pub const MAX_FAILOVER_HOPS: usize = 2;
pub const FAILOVER_BUDGET_MS: u64 = 120_000;
// 语义（审查 A/BLOCKING-1，冻结）：FAILOVER_BUDGET_MS 是**逐 attempt 的 deadline**，
// 不是「换手前查一次」的闸门：每次候选 attempt 前重算 remaining，
// per-attempt 超时 = min(requestTimeoutMs, remaining)；remaining<=0 ⇒ 立即 exhausted。
// 由此冻结同步路径最坏 wall-clock 上界（禁止宣称更小）：
//   worst_sync = requestTimeoutMs × decisionMaxAttempts + backoff_upper(7.5s) + FAILOVER_BUDGET_MS
//   默认值 = 45s×5 + 7.5s + 120s = **352.5s**

pub struct ProviderFailoverChain {
    pub selections: Vec<ResolvedProviderSelection>,
}
impl ProviderFailoverChain {
    pub fn new(selections: Vec<ResolvedProviderSelection>) -> Self;   // 截断到 MAX_FAILOVER_HOPS
    pub fn is_empty(&self) -> bool;
}

impl ProviderManager {
    pub fn new(config: ResolvedProviderSelection) -> Self;                        // 冻结：policy=default, chain 空
    pub fn with_policy(self, policy: UpstreamPolicy) -> Self;                     // builder
    pub fn with_failover(self, chain: ProviderFailoverChain) -> Self;             // builder
    pub fn upstream_policy(&self) -> &UpstreamPolicy;
    pub fn failover_chain(&self) -> Option<&ProviderFailoverChain>;
}
/// 遗留 #4：冷却复位生产入口（清空全表，返回被清条目数）
pub fn reset_global_cooldowns() -> usize;
```

### 3.2 触发类（封闭值域）

`failover_eligible(err) == true` 当且仅当 `classify_error_class(err)` ∈ `{rate_limited, quota, timeout, network, auth}`
**或** `upstream_code_for_hints(err) == Some(ErrorCode::UpstreamServerError)`（`server_error` 只在码表层，不进 `error_class` 词表）。
`bad_request` / `other` **不换手**。

**M1（词表机器锁，审查 B/BLOCKING-2）**：`classify_error_class` 的 7 值封闭集必须有可执行锁——
`pa172_legacy_closure_contract.rs::error_class_wordlist_is_locked` 对 5xx 样本断言返回值恒 ∈ 7 值集且永不为
`"server_error"`（否则「不得加 server_error」只是文档约定；pa171 冻结真值表 0 个 5xx 用例锁不住它）。

**M2（HTTP 500 边界，如实登记）**：既有码表（`retry.rs:334/339`）把 **HTTP 500 冻结为 NonRetryable**，
`UpstreamServerError` 只覆盖 502/503/504/408/520-524/527/530 ⇒ **500 不触发换手**（对 500 换 provider 无收益）。

### 3.3 边界（封闭，测试逐条锚定）

| 编号 | 规则 |
|---|---|
| F-a | 最多 `MAX_FAILOVER_HOPS = 2` 次换手（即候选链最多尝试 3 个 provider：主 + 2） |
| F-b | 每条候选换手前查冷却：`ProviderCooldownRegistry` 命中 ⇒ **跳过、0 次 HTTP**，落 `provider_failover_skipped_cooldown` |
| F-c | 换手预算 `FAILOVER_BUDGET_MS = 120_000`，**从第一次换手起计时**，且为**逐 attempt deadline**（per-attempt 超时 = min(requestTimeoutMs, remaining)；remaining≤0 ⇒ 立即 exhausted）。契约上界 `worst_sync = 352.5s`（默认值） |
| F-d | 每条候选使用**独立的** `RetryBudget`。**位置冻结**：hop 循环必须在 `retry_provider_scoped_with_report`（`:4867`）**调用之外**，每候选各调用一次（budget 在函数内创建 `:4896`；把换手塞进协议 match 闭包 `:922` 会让 3 个 provider 共用同一 budget 与 5 次上限 ⇒ F-d 静默失效） |
| F-e | 全链耗尽 ⇒ 返回**最后一次失败的结构化错误**（既有 `extract_provider_error_detail`（`:4940`）处理后的文本，**不承诺**原始响应 body——既有重试编排早已截断/换头，审查 A/ADVISORY）+ `append_failover_exhausted_marker(err, hops)`，并落 `provider_failover_exhausted` |
| F-f | 成功换手 ⇒ 落 `provider_failover_switched`；**不新增 wire 字段**，成功轮归因语义不变 |
| F-g | **流式入口不参与换手**（F27）。`SingleHopRetryReport`（`retry.rs:260-266`）不携带 `streamed` 信息、`streamed_any_delta` 是入口内局部量（`:1004/:1228`）⇒「已发 delta」在换手层不可判定；流式失败按既有语义上抛，**0 次额外 HTTP** |
| F-h | 主 provider 无 key（`api_key.is_none()`）**不换手**（属配置错误，快速失败；保持既有错误文案） |

### 3.4 参与换手的入口（封闭清单）

| 入口 | 参与 | 说明 |
|---|---|---|
| `decide_with_tools`（`:897`，决策同步） | **是** | 无增量面；hop 循环在 `retry_provider_timeout("decision")` 外层 |
| `continue_with_tool_result`（`:1050`，followup 单发同步） | **是** | OpenAI 分支吞错成 `Ok(local_fallback)`（F12）⇒ 该分支天然不换手（known limitation）；Anthropic 分支（`:1126` 裸调）参与 |
| `continue_with_tool_results`（`:1350`，批量 followup 同步） | **是** | 无增量面 |
| 压缩摘要 `decide_with_tools`（`turn_prep.rs:66`） | **是（继承同一 manager 链）** | 审查 A/BLOCKING-8：该 path 的 provider 就是 §3.5 装链的 manager，标「否」与实现矛盾 ⇒ 参与（同步入口，安全） |
| `decide_with_tools_stream_bounded`（`:974`，决策流式） | **否** | F27：无流式总 deadline ⇒ 换手会把最坏 wall-clock 放大到 30min 级；deferred（需先立流式总 deadline） |
| `continue_with_tool_result_stream_bounded`（`:1169`，followup 流式） | **否** | 同上；该入口 OpenAI 两分支已发 delta 后重发同步请求并吞成 `Ok`（`:1275-1321`），F-g 不可判定 |
| 同步轮 `plan_turn`（`turn_prep.rs:204/216`） | 否 | deferred（见 §10） |

### 3.5 装配（写域 `runtime/tool_exec.rs`、`runtime/compaction_ops.rs`）

`resolve_provider()`（`tool_exec.rs:6-18`）与 `compaction_ops.rs:99-105` 改为：

```rust,ignore
let chain = self.provider_resolver.resolve_failover_chain(&selection);
ProviderManager::new(selection)
    .with_policy(self.provider_resolver.upstream_policy())
    .with_failover(ProviderFailoverChain::new(chain))
```

`ProviderManager::new` 的语义冻结（policy default + 空链）⇒ **所有既有测试构造点零改动**（F9）。

**W1（生产注入必须闭环，审查 A/BLOCKING-7）**：生产装配用的是 `DynamicProviderResolver`（`bin/tui.rs:2884`，
注入点 `:3405`），它**只实现** `resolve_provider_selection` ⇒ 默认方法令 policy=default、链恒空 ⇒ **换手在生产永久不触发**。冻结：
- `DynamicProviderResolver` 必须覆写 `upstream_policy()` 与 `resolve_failover_chain()`，转发内部 `ProviderRegistryStore`（写域 t-w-core）；
- `ProviderRegistryStore`（`agent/config.rs`）必须实现这两个方法（写域 t-w-provider）；
- 判据：`grep -n "fn resolve_failover_chain" crates/pony-agent-core/src/bin/tui.rs` 必须命中。

## 4. P5：可观测（结构化日志 = 唯一面，metrics 结构性 N-A）

| # | 事件名（逐字） | 级别 | 封闭字段 | 唯一发射点 |
|---|---|---|---|---|
| 1 | `provider_failover_switched` | warn | `from_provider` / `to_provider` / `error_class` / `hop` / `elapsed_ms` | 换手成功后、发起候选调用前 |
| 2 | `provider_failover_skipped_cooldown` | warn | `provider_name` / `model` / `remaining_ms` / `hop` | F-b 跳过点 |
| 3 | `provider_failover_exhausted` | warn | `hops` / `last_error_class` / `attempted_providers[]` | F-e |
| 4 | `provider_cooldown_reset` | info | `cleared_entries` | `reset_global_cooldowns()` |
| 5 | `upstream_policy_clamped` | warn | `field` / `provided` / `clamped_to` / `bound`（单字段一次事件） | C2-b |
| 6 | `failover_candidate_ignored` | warn | `provider_id` / `reason`（`unknown|duplicate|self|truncated`） | C2-c |

禁止夹带响应正文/密钥；`trace_id` 由 `log_json_with_fields` 自动带。**不新增 enforcement_point**（仍属 tracing 维度）。

## 5. 冷却遗留收口（#3/#4/#5）

- **R1（#3）锁中毒 = fail-open 显式声明 + 锚点**：4 处 `into_inner()`（F13）**保持** fail-open；契约声明理由「临界区仅 `HashMap` get/insert/retain/remove，poison 后继续可接受；fail-closed 会让一次历史 panic 永久冻结该 provider」。
  机械锚点需要可注入 poison 的接缝，冻结为**仅 debug 构建可见**的测试钩子：
  ```rust,ignore
  #[cfg(debug_assertions)]
  #[doc(hidden)]
  pub fn poison_registry_for_test();   // 起一个线程持锁并 panic，制造 poisoned Mutex
  ```
  锚点用例：`poison_registry_for_test()` 之后 `enter`/`remaining`/`clear` 仍可用（不 panic、不返回毒化错误），并断言 poison 已被吞掉（`registry.remaining("k") == None` 可正常计算）。
- **R2（#4）复位入口接线**：新增 `pub fn reset_global_cooldowns() -> usize`（清空全表、返回条目数、落 `provider_cooldown_reset`）；**生产调用点 = 仅显式 `/clear` 命令分支**（`bin/tui.rs` 中 `/clear` 命中后调用 `state.reset_session_scope()` 的那一处，`:9873` 附近），**不得**挂在 `reset_session_scope` 函数体内（审查 A/BLOCKING-9 + B/ADVISORY：该函数有 6 个调用点 `7021/7754/7861/8882/8884/9873`，覆盖启动 resume、队友切换、返回主会话、选择器 resume ⇒ 一个会话的 /resume 会清掉**进程级**冷却表、把别会话正在冷却的 provider 重新点燃）。D69-16 已按本条修订。reset 只承诺「清空调用时刻的条目集合」，`provider_cooldown_reset` 增字段 `scope`（固定 `"clear"`）。`ProviderManager::cooldown_remaining()` 增生产读取点：冷却期内的 provider 在 TUI 模型视图/状态行可读（若本轮不接 UI，则至少由 `reset_global_cooldowns` + `cooldown_remaining` 在 `provider_failover_skipped_cooldown` 路径被消费 ⇒ 必须有 ≥1 生产调用点，禁止再留"零调用点"）。
- **R3（#5）键归一**：
  ```rust,ignore
  pub(crate) fn cooldown_key_for(sel: &ResolvedProviderSelection) -> String;
  fn escape_cooldown_segment(s: &str) -> String;   // 单射：'%' -> "%25" 先、'|' -> "%7C" 后；空串 -> "~empty~"
  ```
  **D69-12（Lead 裁决）**：转义必须**单射**——否决 `-` 作空段占位（与字面 `-` 不可区分，NFR 审查已指出）；
  `%` 必须先转义再转 `|`，否则 `%7C` 字面量会与真 `|` 碰撞。转义顺序：`%`→`%25` → `|`→`%7C` → 空段→`~empty~`。
  归一真值表：`base_url` 先**去尾斜杠 + scheme/host 小写化**（path 保持原样；审查 B：host 大小写不敏感，`http://API.Example.com/v1` 与 `http://api.example.com/v1` 必须同键，否则冷却可被纯大小写改动绕过）；四段各自 `escape_cooldown_segment`。锚点用例：含 `|` 的两个不同选择不得碰撞；`http://a/v1` 与 `http://a/v1/` 视为同一 key；大小写**不归一**（base_url 路径区分大小写，登记理由）。

## 6. TS 契约：`errorClass` 进门禁（#2）

- **T1**：`tests/ts_contract_snapshot.rs` 夹具构造 `TurnStreamEvent` 时 `error_class: Some("rate_limited".into())`（当前 `:90` 为 `None`），使 `errorClass` 真实上线缆；`UPDATE_CONTRACT_SNAPSHOT=1` 重生成 `docs/contracts/ts-contract.snapshot.json`，快照 `TurnStreamEvent.serdeKeys` 必须含 `errorClass`（38 → 39 键）。
- **T2**：`src/types/runtime.ts` 的 `TurnStreamEvent`（`:1179`）与 `TurnTraceRecord`（`:795`）必须声明 `errorClass?: string`（可选，与 `#[serde(default, skip_serializing_if)]` 对齐）。
- **T3（红绿方向，冻结）**：T1 落地而 T2 未落地时，`node scripts/check-ts-contract.mjs` **必须非零退出**（脚本报 "Rust 有、TS 无"）；T2 落地后 Exit 0。此条为 #2 的真红相。
- **T4**：`TraceTimelineEntry` 无 `errorClass` 字段（Rust 侧无该字段）⇒ 前端类型也不加，避免伪造。

## 7. 租约机械锚点（#11/#12）与 outcome 归约

### 7.1 抽库（写域：新增 `agent/goal_lease.rs`、`bin/tui.rs`）

```rust,ignore
// crates/pony-agent-core/src/agent/goal_lease.rs
pub type GoalRoundLoopLeaseRegistry = std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>;
pub fn new_goal_round_loop_lease_registry() -> GoalRoundLoopLeaseRegistry;
pub struct GoalRoundLoopLease { /* registry, session_id */ }
impl GoalRoundLoopLease {
    /// 抢不到（同会话已有循环）⇒ None。
    pub fn try_acquire(registry: &GoalRoundLoopLeaseRegistry, session_id: &str) -> Option<Self>;
    pub fn session_id(&self) -> &str;
}
impl Drop for GoalRoundLoopLease { /* remove(&session_id) */ }
```
- 语义**逐字不变**（按 `session_id` 分区、`insert` 判互斥、RAII Drop 释放含 panic 展开）；doc 首句改写为 `Arc<Mutex<HashSet<String>>>` 按会话分区（#12 陈旧注释）。
- `bin/tui.rs` 改为 `use pony_agent_core::agent::goal_lease::{...}` 并删除本地定义（F17 引用点：`:8155`/`:8164`/`:8474-8475`/`:10032`）。

### 7.2 outcome 归约可测化

`control_plane::turn_stream_outcome_from_terminal`（F18）提升为 `pub`（或新增同等 `pub` 包装），签名冻结：
```rust,ignore
pub fn turn_stream_outcome_from_terminal(terminal: /* 既有入参 */) -> TurnStreamOutcome;
```
真值表（10 例，测试逐行锚定，**以 HEAD 实现为准**）：决策五臂 `Fail⇒Failed{error}` / `Cancel⇒Cancelled` /
`WaitUser⇒Suspended` / **`Pause⇒Cancelled`**（实现 `control_plane/mod.rs:705-710` 明确写「用户停止被折成 Pause ⇒ Cancelled」） / `Continue|Complete⇒按 phase`；phase 五臂
`failed⇒Failed` / `cancelled⇒Cancelled` / `suspended⇒Suspended` / `completed⇒Completed` / 其它⇒`Unknown`。

## 8. TUI 遗留收口（#8/#9/#10/#13 + 两处裸 break）

| 编号 | 契约 |
|---|---|
| B1（#13） | `UiEvent::GoalRoundsStopped { reason, status_line, session_id: Option<String> }`（additive）。事件循环：`session_id` 存在且 ≠ 当前会话 ⇒ **只落 `goal_rounds_stopped` 日志、不改状态行、不进消息区**；`None` ⇒ 按旧行为（当前会话）处理 |
| B2（#10） | `TuiState.goal_rounds_stopped` 必须在 `reset_session_scope` 清 `None`，且**必须有读取点**：横幅/状态行派生「已停止（{visible_line}）」时读它（禁止只写不读） |
| B3（#8） | `turn_loop_in_flight` 由单 bool 改按 `session_id` 分区（`HashSet<String>` + RAII guard）；提前 return 只释放**本会话**，不误清另一会话在飞语义；读取点 `:10703` 语义改为「当前会话是否在飞」。锚点：若该类型可通过既有 bin-as-module 模式（`#[path = "../src/bin/tui.rs"] mod tui;`）在测试中触达则补双会话用例；**不可触达则如实标 `non-machine（靠 review）` 并登记，禁止为测试而把内部类型改成 `pub`**（避免测试倒逼 API 泄漏） |
| B4（#9） | `GoalRoundStopReason::BudgetPersistFailed`（additive 第 8 变体）`visible_line()` 逐字冻结为：`goal 续轮已停止（本轮预算推进落盘失败，已暂停自动续轮）`；`tui.rs:8265-8275` 改用它（不再复用 `BudgetExhausted`）。`goal_rounds_stopped` 日志 `reason` 取值扩为 `budget_persist_failed` |
| B5（#23/F23） | `tui.rs:8187`（无 active goal）与 `:8191`（有待作答让位）两处 break 必须补可见原因 + 结构化日志。冻结：无 active goal ⇒ 新增 `GoalRoundStopReason::NoActiveGoal`，文案 `goal 续轮已停止（当前会话无进行中的目标）`，日志 reason tag `no_active_goal`；有待作答 ⇒ **不新增变体**（既有让位语义），但必须落日志 `goal_rounds_yield_pending_requests`（`session_id` / `pending_requests`），状态行仍由既有 Ask/审批面板承担 |
| B6（#21 相关） | `control_plane/mod.rs:1947`、`:2756` 两处 `Err(_) => return Ok(response)` 静默吞预算推进失败 ⇒ 改为落 `warn` 日志 `goal_round_advance_failed`（字段 `stage` ∈ {`after_turn`,`planner`}），**行为不变**（仍返回 Ok，不改调度语义），仅消除静默 |

> 说明：`BudgetPersistFailed` 与 `NoActiveGoal` 同为 additive 变体，**不得**被 `goal_round_stop_reason`（三值真值表）产出。

## 9. 写域、依赖与合并顺序

| 契约 | 文件 | 归属任务 | 依赖 |
|---|---|---|---|
| §2 配置面/钳制 | `agent/config.rs`（含 `ProviderRegistryStore::upstream_policy()` / `resolve_failover_chain()` 实现） | t-w-provider | 无 |
| §3 换手/超时/重试 | `agent/provider/mod.rs`、`agent/retry.rs` | t-w-provider | §2 |
| §3.5 装配 | `runtime/tool_exec.rs`、`runtime/compaction_ops.rs` | t-w-provider | §2/§3 |
| §5 冷却收口 | `agent/provider/mod.rs`、`agent/retry.rs`、`bin/tui.rs`（`reset_global_cooldowns` 调用点） | t-w-provider（provider 侧）+ t-w-core（tui 调用） | 函数签名冻结 ⇒ 可并行 |
| §6 TS 契约 | `tests/ts_contract_snapshot.rs`、`docs/contracts/ts-contract.snapshot.json`（t-red）+ `src/types/runtime.ts`（t-w-core） | t-red / t-w-core | 测试先冻结 |
| §7 租约抽库 | `agent/goal_lease.rs`、`bin/tui.rs`、`agent/control_plane/mod.rs` | t-w-core | 无 |
| §8 TUI 收口 | `bin/tui.rs`、`agent/goal.rs`、`agent/control_plane/mod.rs` | t-w-core | §7 |
| §3.5 W1 生产注入闭环 | `bin/tui.rs`（`DynamicProviderResolver` 覆写两个新 trait 方法） | t-w-core | §2 |
| 收口文档 | `docs/observability.yaml`、本矩阵、ADR、任务卡、看板 | Lead | 全部 |

**越界判定**：任何任务写域外的文件改动（编译衍生除外）判 FAIL。`tests/pa171_*.rs` 四文件为冻结只读。

## 10. PA-171 遗留逐条闭合表

| # | 遗留项 | 闭合动作 | 承载 | 判据（机器） | contract_status | impl_status |
|---|---|---|---|---|---|---|
| 1 | 跨 provider 故障转移 + 超时/重试配置化 | 本卡 P1–P5（**仅同步入口**，流式 deferred） | provider/config/retry/tool_exec/compaction_ops | `pa172_provider_failover_contract` + `pa172_upstream_policy_config_contract` 全绿 | closed | planned |
| 2 | `errorClass` 未进 TS 契约/前端类型 | §6（T1 快照重生成 + T2 前端 + M3 TurnTraceRecord 自锁） | ts-contract / `src/types/runtime.ts` | `cargo test --test ts_contract_snapshot` + `npm run contract:check` + `turn_trace_record_serializes_error_class` | closed | planned |
| 3 | 冷却 `into_inner` fail-open | 显式声明 + poison 锚点 | provider/mod.rs | §5 R1（`provider::poison_registry_for_test()`） | closed_with_declaration | planned |
| 4 | `clear_cooldown`/`cooldown_remaining` 零生产调用点 | `reset_global_cooldowns()` + **/clear 分支**接线（R2） | provider/mod.rs + bin/tui.rs | 生产调用点存在（grep 判据）+ `provider_cooldown_reset` 日志 | closed | planned |
| 5 | 冷却键未转义/未归一 | §5 R3（单射转义 + scheme/host 小写 + 去尾斜杠） | provider/mod.rs | 键归一真值表单测 | closed | planned |
| 6 | `now_unix_ms` 死参 | `rate_limit_recovery_hint_at` + `user_facing_error_summary_at` 消费 now | retry.rs + bin/tui/errors.rs | 同一错误不同 now ⇒ 不同剩余窗口文案 | closed | planned |
| 7 | 租约曾为进程级单 bool | wave-68-a2 已按会话分区；本轮随抽库改写 doc | agent/goal_lease.rs | doc 首句与实际类型一致（**non-machine，靠 review**） | closed | planned |
| 8 | `turn_loop_in_flight` 单 bool | B3 按会话分区 | bin/tui.rs | 可触达则双会话用例；不可触达 ⇒ **non-machine（靠 review）**，禁止为测试把内部类型改 pub | closed | planned |
| 9 | 落盘失败复用 `BudgetExhausted` 文案 | B4 新变体 | agent/goal.rs + bin/tui.rs | `visible_line` 逐字断言 + 不等于 BudgetExhausted 文案 | closed | planned |
| 10 | `goal_rounds_stopped` 只写不读、未清 | B2 | bin/tui.rs | 复位后为 None + 读取点存在（单测/审查） | closed | planned |
| 11 | 租约与五臂归约缺可执行用例 | §7 | `tests/pa171_lease_anchor_contract.rs` | 用例全绿（红相先红） | closed | planned |
| 12 | 租约类型 doc 陈旧 | §7.1 | agent/goal_lease.rs | doc 首句含 `HashSet`/按会话（review） | closed | planned |
| 13 | `GoalRoundsStopped` 无 session_id | B1 | bin/tui.rs | 变体字段 + 跨会话不污染（可触达则用例，否则部分闭合并登记） | closed | planned |
| 14 | a2 计划复审由 Lead 定点复核替代 | 本波 2 路独立 Subagent 计划审查 + 修订后 round2 复审 | `.dev-team/review-plan-wave69-v2-*.json` | `dev-review-lint.py` Exit 0 + verdict=**PASS** | **pending_review_pass** | planned |

> **状态语义（审查 B 指出并已修正）**：`contract_status` 只表示"契约层是否已定义闭合判据"；`impl_status` 一律 `planned`，
> 直到实施任务交付并跑出机器收据才由 Lead 改 `implemented`。**禁止**把 `contract_status=closed` 读成"已实现"。
> #14 在 round2 审查 PASS 之前保持 `pending_review_pass`（round1 两路均 FAIL）。

## 11. 偏差与已知限制（如实登记）

1. **矩阵作者 = Lead**（非 Protocol Agent）：Protocol Agent 首次尝试耗尽上下文未产出，Lead 按"规划产物允许 Lead 落盘"直接接管；NFR 基准仍由 Protocol Agent 产出并过 lint。**非独立撰写，如实登记**。
2. **基线编译环境陷阱**（F26）：所有门禁命令必须带 `--target-dir target-test` / `--target-dir target-check`；裸 `cargo test` 会链接到其它 checkout 的陈旧 rlib 产出假红。此陷阱写入账本与任务卡断点指引。
3. **同步轮 `plan_turn`（`turn_prep.rs:204/216`）不参与换手**；**压缩摘要路径（`turn_prep.rs:66`）参与**（同一 manager 链，见 §3.4，审查 A/BLOCKING-8 纠正）。⇒ 换手覆盖面 = 4 个同步入口 + 1 个压缩摘要路径；流式与 `plan_turn` 未见覆盖，登记 **deferred**。
4. **OpenAI followup 吞错**（F12）：该分支失败被转成本地兜底 `Ok`，外层无 Err ⇒ 换手不触发。**不改**该语义（属既有行为），登记 known limitation。
5. **成本/遥测口径**：一手换手会发出 2 次 HTTP 而只记 1 条 `InitialRequest`（`turn_stream.rs:2862-2876`），可能低估成本。登记 deferred。
6. **流式入口不参与换手**（F27/§3.4）：流式无总 deadline，换手会把最坏 wall-clock 放大到 30min 级 ⇒ 本波只覆盖同步入口。已知限制，非目标裁剪，登记 deferred（需先立"流式总 deadline"）。
7. **HTTP 500 不触发换手**（M2）：既有码表冻结 500 为 NonRetryable；本波不破该契约。
8. **"最后一次错误"不是原始 body**（F-e）：既有 `extract_provider_error_detail` 已截断/换头，契约只承诺该结构化文本 + `failover_exhausted=` 标记。
9. **`retryBudgetMs` 下界抬到 1000**（审查 B/ADVISORY）：0 预算语义与 `RetryBudget::is_exhausted` 的 `>=` 判据冲突，收紧边界消歧义。
10. **`dev-stub-lint.py` 不适用 Rust**（沿用 PA-171 矩阵 §8 偏差声明 1）：本波不以其作为门禁，编译门禁 + 矩阵冻结签名替代。

## 12. 收据（本矩阵的机器凭证）

| 门禁 | 命令 | 结果 |
|---|---|---|
| 基线提交 | `git rev-parse --short HEAD` | `458b952e` |
| 基线 lib 测试 | `cargo test -p pony-agent-core --lib --target-dir target-test` | `1404 passed` |
| 基线 tui 回归 | `cargo test -p pony-agent-core --test tui_batch2_regression --target-dir target-test` | `96 passed` |
| 基线冻结测试 | 4 个 `pa171_*` + adversarial | 由 t-red 阶段复核（见 `.dev-team/evidence-wave69/baseline-tests.log`） |
| 假红复现（陷阱证据） | `cargo test --test tui_batch2_regression`（默认 `target/`） | `E0599 LoopAlreadyOwned`（陈旧 rlib） |
| TS 契约现状 | `node scripts/check-ts-contract.mjs` | Exit 0（**对 errorClass 失明**，见 §6 T3） |
| NFR 基准 lint | `python3 ~/.agents/skills/dev-team/scripts/dev-nfr-lint.py .dev-team/nfr-baseline-wave69.json` | `[PASS] … 校验通过`，**Exit 0**（Protocol Agent 实测，2026-11-09） |
| 计划审查 round1 | `dev-review-lint.py .dev-team/review-plan-wave69-{a,b}.json` | 两份均 **Exit 0**、verdict = **FAIL**（5+5 BLOCKING，已按修订闭环，见 §11 与 round2） |

**Amendment 记录**：本矩阵对 `.dev-team/contract-matrix-pa171.md` 的追加见该文件 `### Amendment: wave-69-a1`。
