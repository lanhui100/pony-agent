# ADR 0104：上游调用时限/重试配置化与跨 provider 有界故障转移

Status: implemented

## 背景

ADR 0103（PA-171）把"一次上游限流"收敛为一次可见失败，但明确把两件事留给后续：

1. **45s 总超时 / 5 次重试 / 30s 退避预算全是硬编码**：`provider/mod.rs:2737`、`:2788` 两处内联
   `Duration::from_secs(45)`；`const PROVIDER_TIMEOUT_RETRY_MAX_ATTEMPTS: u32 = 5`（`:4635`）；
   退避表 `:4771-4779`（500ms 起 ×2、上限 8s、sleep 预算 30_000ms）。仓内无 env、无配置通道，
   想改只能改代码重编译。
2. **没有任何跨 provider 换手能力**：`ProviderManager` 由单值 `ResolvedProviderSelection` 构造
   （`runtime/tool_exec.rs:17`、`compaction_ops.rs:103`），`providers.json` 只有
   `providers[] + selectedProviderId`（`config.rs:212-215`），选择算法是单值三级回落
   （`config.rs:384-400`）。现有"回退"全部是**同 provider 内降级**：
   followup 本地兜底（`provider/mod.rs:5092-5094`）、流式→同步重发（`:1279-1317`）、
   `tool_recovery.rs:471-475`；`RecoveryPipeline`（`recovery.rs:97-101`）生产未接线。

诊断会话 `pa_01a1ba70-…` 的实际后果：一个 provider 先被打 45s 总超时杀死 5 轮，随后撞上
5 小时配额窗口 ⇒**该 provider 在整个窗口内不可用，而用户只能手工换模型**（10:43 才恢复）。
这是可机械化的动作被留给了人。

## Decision

1. **配置面 additive 落在 `providers.json` 顶层**（`agent/config.rs`）：
   `upstreamPolicy { requestTimeoutMs, decisionMaxAttempts, retryBudgetMs }` 与
   `failoverProviderIds: [provider_id, …]`（**有序显式候选链**）。
   缺省值 = 今天的真实值（45000 / 5 / 30000 / 空链）⇒ 未配置时行为零变更；
   配置面必须能被读写 round-trip（前端保存不得丢字段）。

2. **越界钳制而非报错**（fail-safe）：`requestTimeoutMs ∈ [15000, 300000]`、
   `decisionMaxAttempts ∈ [2, 10]`、`retryBudgetMs ∈ [5000, 300000]`（下界由 PA-173 上调，
   见文末「PA-173 修订」）；越界钳到边界并落
   `warn` 结构化日志 `upstream_policy_clamped`（字段 `field`/`provided`/`clamped_to`/`bound`）。
   **钳制只作用于「生效策略」解析（`ProviderRegistryStore::upstream_policy()`），不写盘**：
   `load_view` / `save_view` 一律按原值往返，磁盘保留用户写入的原值。
   未知/重复/等于自身的候选 id 忽略并 `warn` `failover_candidate_ignored`。
   理由：配置文件是用户可手改面，一处越界不该让整个 provider 面失效（fail-closed 会让
   TUI 直接无法发起任何模型调用）。
   **附加（计划审查 B/BLOCKING-1 后追加）**：`#[serde(default)]` 只补缺字段、**接不住类型错**，
   而 `providers.json` 是整体 `from_str`（`config.rs:498-508`）⇒ 一句 `"retryBudgetMs":"30s"`
   会让整文件解析失败并回落 `default_registry()`，把用户全部 provider 配置顶替。故新增字段
   一律**逐字段容错反序列化**（接受 number/数字字符串/null/其它 ⇒ 回落默认 + warn），
   新增字段的任何畸形都不得使既有 provider 配置整体失效。
   `retryBudgetMs` 下界从 0 抬到 1000：0 预算与既有 `RetryBudget::is_exhausted` 的 `>=` 判据
   （`retry.rs:224`）语义冲突，边界收紧以消歧义。（该下界已由 PA-173 再上调到 5000，见文末。）

3. **有界故障转移（换手，仅同步入口）**：当一次**同步**模型调用失败、且
   `failover_eligible(err)`（= `classify_error_class` ∈ `{rate_limited, quota, timeout, network, auth}`
   ∪ `upstream_code_for_hints == UpstreamServerError`）时，按 `failoverProviderIds` 顺序换到下一个
   provider 重试**同一次调用**：
   - 最多 `MAX_FAILOVER_HOPS = 2` 次换手；**hop 循环位于 `retry_provider_scoped_with_report` 调用之外**
     （每候选独立 `RetryBudget`，否则 3 个 provider 会共用同一 5 次/30s 预算 ⇒ "有界"变假）；
   - `FAILOVER_BUDGET_MS = 120_000` 是**逐 attempt 的 deadline**（not 换手前查一次）：
     每次候选 attempt 前重算 `remaining`，per-attempt 超时 = `min(requestTimeoutMs, remaining)`，
     `remaining <= 0` 立即 exhausted。由此冻结同步最坏 wall-clock：
     `45s×5 + 7.5s + 120s = 352.5s`（默认值，登记为契约上界，不得宣称更小）；
   - 候选若**处于冷却期** ⇒ 跳过、**0 次 HTTP**；
   - 全链耗尽 ⇒ 返回**最后一次失败的结构化错误**（既有 `extract_provider_error_detail` 处理后的文本，
     不承诺原始 body）+ 可解析后缀 `failover_exhausted=<hops>`，禁止伪造成功或吞掉失败；
   - `bad_request` / `other` **不换手**；**HTTP 500 不换手**（既有码表冻结 500 为 NonRetryable，
     换 provider 对 500 无收益）；
   - **流式入口不参与换手**：流式只有 15s connect + 180s read-idle、**无总 deadline**，单跳最坏约 907s，
     叠加 2 跳会把最坏 wall-clock 放大到 30min 级；而 `SingleHopRetryReport` 不携带 `streamed` 信息，
     "已发 delta 不得重发"在换手层不可判定。⇒ 流式换手登记 deferred（前置项：先立"流式总 deadline"）。
   - **生产注入必须闭环**：`DynamicProviderResolver`（`bin/tui.rs:2884`）必须覆写
     `upstream_policy()` 与 `resolve_failover_chain()`（转发 `ProviderRegistryStore`），否则 trait 默认实现
     会让链恒空、换手在生产永久不触发（"编排面已交付但未接线"）。

4. **观测面唯一化**：metrics 是结构性 N-A（`docs/observability.yaml:45-56`：桌面/TUI 无常驻
   监听地址、无 `/metrics`），故只新增封闭字段的 JSON 日志事件
   `provider_failover_switched` / `provider_failover_skipped_cooldown` /
   `provider_failover_exhausted` / `provider_cooldown_reset`。**不新增 wire 字段**：
   换手不改变成功轮的既有归因语义（`provider_source/provider_mode` 仍表示轮次模式，
   降级真相由日志 + 被冷却冻结的首个 provider 共同承载）。这是**刻意**的收敛——
   PA-171 的 `fallback_reason` 是封闭三字面量域，本轮不为换手扩域。

5. **冷却面收口（PA-171 遗留 #3/#4/#5）**：
   - 锁中毒保持 **fail-open 并显式声明**（临界区仅内存 map 操作；fail-closed 会让一次历史 panic
     永久冻结该 provider，而当时根本没有复位入口），并补 `#[cfg(debug_assertions)]` 的
     `poison_registry_for_test()` 作为可执行锚点；
   - 新增生产复位入口 `reset_global_cooldowns() -> usize` + `provider_cooldown_reset` 日志，
     **只挂在显式 `/clear` 命令分支**（不挂 `reset_session_scope` 函数体：它有 6 个调用点，
     会让一个会话的 /resume 清掉进程级冷却表、把别的会话正在冷却的上游重新点燃）；
   - `cooldown_key()` 四段改为**单射段内转义 + base_url 归一**（`%`→`%25` 先、`|`→`%7C` 后、
     空段→`~empty~`、去尾斜杠、scheme/host 小写化、path 保持原样）⇒ 消除 `|` 碰撞、尾斜杠变体与
     host 大小写绕过。

6. **错误时间真正参与文案（遗留 #6）**：`user_facing_error_summary_at(err, now_unix_ms)` 必须
   消费 `now`（新增 `retry::rate_limit_recovery_hint_at`），使"预计 Ns 后恢复"随真实时刻收敛，
   而不是把头文件里的死参丢掉。

7. **`errorClass` 进 TS 契约门禁（遗留 #2）**：`docs/contracts/ts-contract.snapshot.json` 必须
   含 `errorClass`（夹具填 `Some(..)` 后重生成），前端 `src/types/runtime.ts` 的
   `TurnStreamEvent` / `TurnTraceRecord` 必须声明 `errorClass?: string`；
   `node scripts/check-ts-contract.mjs` 在"前端未加字段"时必须**非零退出**。
   门禁红绿方向由冻结测试与脚本双向锚定。

8. **租约锚点化（遗留 #7/#11）**：`GoalRoundLoopLease` 从 `bin/tui.rs` 抽到 lib 模块
   `agent/goal_lease.rs`（纯类型、语义不变），`control_plane::turn_stream_outcome_from_terminal`
   提为可测公共面 ⇒ 得以在 `tests/` 层用集成测试锚定"按会话分区/互斥/Drop 释放/双会话并发"
   与 outcome 归约的 5 决策臂 × 5 phase 臂真值表。抽库替代"在 bin 内写 `#[cfg(test)]`"的理由：
   bin 单测无法被跨模块复用，也没法在 `cargo test --test` 层面作为冻结验收面。

## Alternatives considered

- **自动健康度评分 / 自动发现候选**（落选）：需要探活流量、状态机与"谁算健康"的裁决，
  属新子系统；显式候选链已能覆盖"主上游限流时换到备份"这一真实痛点，且可预测、可解释。
- **把 failover 做进 `RecoveryPipeline`**（落选）：该类生产未接线（仅 tests 引用），
  接线本身就是大改动；且它面向"轮内恢复策略"，与"单次调用换手"层级不同。
- **失败即换、不查冷却**（落选）：会在冷却窗口内把已 429 的 provider 再打一遍，
  直接反噬 PA-171 的 C3（冷却期 0 次 HTTP）；换手必须尊重冷却。
- **流式入口也参与换手**（本轮落选，登记 deferred）：流式无总 deadline（15s connect + 180s read-idle），
  单跳最坏 ~907s；先换手等于把"一次失败"变成"半小时等待"，且已发 delta 无法安全重放。
  前置项：流式总 deadline 立项（另案）。
- **换手结果写新 wire 字段（如 `failoverHops`）**（落选）：会扩 PA-171 已冻结的
  归因契约面；本轮信息价值可完全由结构化日志承载，等有消费方时再议。
- **配置非法即报错/拒绝启动**（落选）：见 Decision 2 理由（用户手改面 fail-safe）。
- **`errorClass` 只加前端不加门禁**（落选）：正是 PA-171 遗留 #2 的现状——门禁对键失明，
  Rust 侧 `skip_serializing_if` 让快照永远是 38 键，前端漏声明也没人拦。
- **冷却锁改 fail-closed**（落选）：见 Decision 5；改为"显式声明 fail-open + 补复位入口"，
  把风险转成可操作能力而不是永久冻结。

## 影响

- 行为：默认配置下**与 PA-171 交付后完全一致**；只有显式配置 `failoverProviderIds` 后才会
  出现换手（且仅在同步入口）。45s/5 次/30s 从"改代码才能改"变成"改配置即可"。
  同步路径最坏 wall-clock 契约上界 352.5s（默认值），流式入口不变。
- 兼容性：全部 additive（新增可选配置字段、新增枚举变体 `BudgetPersistFailed`、
  新增 `UiEvent::GoalRoundsStopped.session_id`、新增公开函数/模块）；
  `ProviderRetryPolicy::classify/decide`、`RetryBudget`、`emit_stream_failed` 14 参签名、
  PA-171 四个冻结测试文件均不变。
- 不可逆面：`providers.json` **新增顶层键**（旧版本读新文件会忽略未知键，不报错）；
  无 DB/序列化编号变更。
- 可观测性：新增 4 个结构化日志事件（字段清单见契约矩阵 §6），不新增 enforcement_point，
  metrics/logs 维度维持结构性 N-A。

## 验证

- 冻结验收（红相先锚定）：`tests/pa172_provider_failover_contract.rs`（双 loopback mock 物理计数：
  主 429 ⇒ 备收到请求、全链失败返回最后错误、冷却跳过 0 次 HTTP、hop 上限、预算上限）、
  `tests/pa172_upstream_policy_config_contract.rs`（默认值不变 / 钳制边界 / 非法值不致命 /
  round-trip 不丢字段）、`tests/pa171_lease_anchor_contract.rs`（租约 + outcome 归约真值表）、
  `tests/ts_contract_snapshot.rs`（含 `errorClass`）+ `node scripts/check-ts-contract.mjs`。
- 回归：`cargo check --all-targets`、PA-171 四个冻结测试、`--lib` 全量、
  `tui_batch2_regression` / `tui_session_resilience_acceptance`、`npm run typecheck`。
- 观测门禁：`audit-diff.sh --base <基线>` 与 `check-declaration.sh` 双 Exit 0。

## PA-173 修订（2026-11-09）

现场事故：TUI 连续多轮 `elapsed=1001ms` 超时 + 「重试已用尽」，`providers.json` 的
`upstreamPolicy` 被钉在 `{1000, 1, 1000}` 三个下界上。诊断结论：上游健康（网关 2571/2585 = 200、
0 个 5xx、p50 2.2s），失败由**配置面三条缺陷叠加**造成。据此修订本 ADR 三处：

1. **钳制不落盘**（本文 §2 修订）：`normalize_storage` 不再回写 `upstream_policy`；
   钳制只发生在 `ProviderRegistryStore::upstream_policy()`（生效策略解析），并保留
   `upstream_policy_clamped` warn。理由：钳制值落盘 = 把「一次越界输入」永久固化成
   「用户意图」，且三个下界当时刚好构成不可用的组合（1s 超时 + 零重试），事故即由此产生。
   `ProviderManager::with_policy` 保持不钳制（冻结测试直接构造策略值）。
2. **下界上调**：`requestTimeoutMs` 1000 → **15000**（实测最小同步请求 1.76s、网关 p50 2.2s、
   模型 p50 4.67s、p99 36s ⇒ 1s 下界物理不可达）；`decisionMaxAttempts` 1 → **2**
   （下界 1 = 合法的「零重试」，与本文 §2 的「越界不致命」目的相反）；
   `retryBudgetMs` 1000 → **5000**（`retryBudgetMs` 是 **sleep-only** 预算：2 次尝试=1 次 500ms
   退避，5 次尝试=500+1000+2000+4000=7.5s；5s 下界覆盖「下界 attempts 组合」的前 4 次退避，
   高 attempts 组合仍由用户显式配置决定，不做隐式截断）。
   默认值 45000 / 5 / 30000 不变，未配置时行为仍零变更。
   **已接受的代价（L3-P 审查 §1）**：同步路径**无跨 attempt 总 deadline**（`attempt_deadline`
   只由换手注入，且换手仅 decision 路径），故批量最坏墙钟 = `attempts × requestTimeoutMs` +
   sleep 预算（默认 5×45s+7.5s ≈ 232s，PA-173 之前为 ≤45s）。与 1:1 followup 的既有语义一致
   （同一 helper、同一乘法），本卡不引入新的取消通道；跨 attempt 总 deadline 单列后续卡。
3. **批量 followup 补齐重试**：`send_openai_tool_followup_batch_request`（多工具批量）此前
   完全无重试层，而 1:1 followup 有；现按同一 `retry_provider_followup_timeout_with_policy`
   包装，label = `followup_sync_batch`，`max_retries = decision_max_attempts - 1`。修复前
   批量路径首跳失败即终态，UI 仍显示「重试已用尽」——文案与事实相反。

**不改文案**：批量路径补齐重试后「重试已用尽」恢复为事实描述；attempts 下界 2 保证任何
合法配置至少发生一次重试。（尝试次数在文案生产点不可得，见 PA-173 决策记录。）

新增/迁移冻结验收：`tests/pa173_upstream_policy_guard_contract.rs`（D1 磁盘原值 + 生效钳制、
D2 下界、D3 批量重试与有界重试）、`tests/pa172_upstream_policy_config_contract.rs`（三处下界断言
+ 极端值语义改为「读取路径钳制、磁盘保原值」）。

## PA-174 修订（2026-11-09，第二轮）

现场症状：「新的轮次开始，没有成功接入之前轮次的历史」。取证后确认是**两个独立缺陷**叠加，
外加三项 PA-173 登记的遗留项，一并收口：

1. **P1 历史重建（主症状；数据面）**：实测会话 `pa_01a1ba70` 的 `session_data` 为 `__index_only`
   且 `history: []`、`normalized_messages` 该会话 0 行、会话日志 0 条 `append_message`，而
   `turn_events` 有 13098 行 / 48 turn —— 历史**只**存在于事件日志。`sqlite_session_commit.rs`
   对「投影侧全空」的行故意跳过 ⇒ 空壳永久保留；TUI 新轮次又传 `history: Vec::new()`。
   决定：新增 `session::history_from_turn_events`（纯函数，按 seq 升序只取 user/assistant
   消息事件），并接入 `SqliteSessionBackend::load_store` 的两条返回路径 ⇒ 折叠历史为空但事件
   日志有消息时**必须重建**，不返回空壳。契约红线：不得凭空造假（无消息事件则保持空）。
2. **P2 角色还原（协议面）**：历史把上一轮 assistant 存为 `Developer + "Previous assistant reply: "`
   标记，而 `to_chat_role` 把 Developer 映成 `system` —— openai-completions / openai-responses
   两条协议**不还原**（仅 Anthropic 还原）。实测 125 条 decision 请求中 75 条携带该标记、单条
   最多 12 条。决定：三条协议统一还原为 `role: assistant` 并剥前缀，顺序不变，system 文本不含
   该字面量。
3. **F1 1:1 followup 重试污染与错误降级**：1:1 三臂补 pristine-base 重置与原始错误保真
   （与 PA-173 批量同构）；Anthropic 1:1 臂补回缺失的重试层。
4. **F2 聚合墙钟（取代本文 §PA-173 修订第 2 条的「无总 deadline」代价条款）**：
   新增 `UPSTREAM_AGGREGATE_DEADLINE_MS_MAX = 180_000` 与
   `aggregate_sync_call_budget_ms = min(timeout × attempts + retry_budget, 180_000)`，
   同步入口（decision / 1:1 / batch 三臂）注入绝对 deadline；退避 sleep 按剩余预算截断
   （`DeadlineSleeper`）。未注入 deadline 的调用者行为逐字不变（既有 sleep 累计断言不动）。
5. **F3 生效策略出口**：新增 `upstream_policy_report(raw, effective)` 与
   `ponycli config effective`（打印 `disk_policy` / `effective_policy` / `clamped_fields`，
   不含任何密钥）⇒ 修掉「钳制只读后生效值无观测出口」的缺口。

冻结验收：`tests/pa174_history_wiring_and_followup_contract.rs`（10 用例：P1×3 / P2 / F1×3 /
F2×2 / F3）。红相：P1 装载侧 `[]` vs 重建历史、P2 角色序列 `[system,system,user,system,user]`、
F1 `5≠3` 与 `1≠2`、F2 未接行为面时 `hits=5` 且 12.76s。
