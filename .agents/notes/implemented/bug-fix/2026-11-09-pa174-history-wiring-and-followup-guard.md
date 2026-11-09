# PA-174 轮次历史接入（P1/P2）+ 三项遗留收口（F1/F2/F3）

Status: implemented

- 日期：2026-11-09
- 关联：ADR 0104「PA-174 修订」/ 红相提交 `017bc2ec` / 实现 `e927dc39`
- 前序：PA-173（钳制不落盘、下界、批量重试）、ADR 0060（turn context 完整性）
- 触发：用户报告「新的轮次开始，没有成功接入之前轮次的历史」

## 决定

1. **P1 从事件日志重建历史**：`session::history_from_turn_events(events) -> Vec<TurnHistoryMessage>`
   （按 seq 升序，只取 `user/message` 与 `assistant/message`，忽略空文本与其它事件）；
   `SqliteSessionBackend::load_store` 的两条返回路径在 fold/投影补全之后调用
   `rebuild_history_from_turn_events_when_projection_empty` ⇒ 折叠历史为空但事件有消息时重建。
   无消息事件则保持空（不造假）。
2. **P2 角色还原三协议统一**：`Previous assistant reply: X` 标记 ⇒ `{"role":"assistant","content":X}`
   （去前缀），openai-completions / openai-responses / anthropic 一致；其余 Developer/System 仍为
   system；顺序不变；system 文本过滤该字面量。
3. **F1 1:1 三臂**：pristine-base 每 attempt 重置 + 原始错误保真（`last_provider_error`）；
   Anthropic 1:1 补重试层（label 保持 `followup_sync`）；`or_else(local fallback)` 行为不变。
4. **F2 聚合墙钟**：`UPSTREAM_AGGREGATE_DEADLINE_MS_MAX = 180_000`、
   `aggregate_sync_call_budget_ms = min(timeout×attempts + retry_budget, cap)`；同步入口注入
   绝对 deadline；`DeadlineSleeper` 把退避 sleep 截断到剩余预算；无 deadline 的调用者逐字不变。
5. **F3 生效策略出口**：`upstream_policy_report(raw, effective)` + `ponycli config effective`
   （`disk_policy` / `effective_policy` / `clamped_fields`，无密钥）。

## 为什么

- P1 的真因不是「装配代码没接历史」——装配链完整（`turn_prep` → `context_builder`），
  而是**持久化三条通道全空**：`session_data` 是 `__index_only` 空壳、`normalized_messages` 0 行、
  会话日志 0 条 `append_message`；写路径当时因单实例锁降级只读（`commit_boundary_failed:
  attempt to write a readonly database`）。唯一完好的数据面是 `turn_events`（13098 行/48 turn）
  ⇒ 只能从事件日志重建，否则重启即失忆。
- P2 是同一症状的第二层：即便历史内容在，上一轮 assistant 以 `system` 角色 + 字面前缀出现，
  角色结构丢失、与 Anthropic 路径分叉（同会话换协议即变语义）。
- F1/F2/F3 是 PA-173 审查登记的遗留，已在其决策记录中明确为「本卡不修、登记后续」；
  本卡按最小闭包逐条收口，F2 用「聚合上界 + 退避截断」而非取消通道，避免改动冻结的 sleep 语义。

## Alternatives considered

- **只做装配层修补（在 `turn_prep` 里塞 fallback 历史）**：数据面仍是空壳，重启依旧失忆；且会与
  投影/折叠路径产生第二份真值源，否决。
- **启动时全量扫描 `turn_events` 重建所有会话**：13098 行/会话 × 142 会话的 IO 与内存放大，
  且启动期已因 fold 超时降级；改为**按需重建**（只处理折叠历史为空的会话），否决全量方案。
- **把重建结果写回 `session_data`**（自愈落盘）：写路径在只读降级时必然失败，且会把「重建值」
  与「折叠值」混成同一真值源；保持只读重建（每次装载时算），否决写回。
- **P2 改成在存储层就存 assistant 角色**（不做 "Previous assistant reply" 标记）：会破坏既有
  `sessions.db` 历史格式与 `session_read_from_log` 冻结断言（回放口径），改为在 provider 层还原，否决。
- **F1 顺手也给 openai/responses 两条臂去掉 `or_else` 本地兜底**：会改变既有降级语义并让
  「上游失败」暴露成硬错误，超出闭包；保持兜底，只修转录与错误保真，否决。
- **F2 用取消通道 / 客户端 timeout 叠加**：会与 per-attempt timeout 形成双重计时并可能触发
  竞态中止；改用「剩余预算」单一来源（复用既有 `effective_attempt_timeout_ms`），否决双计时。
- **F2 直接把 180s 写死进每个调度点**：重复真值源；改为 `aggregate_sync_call_budget_ms` 单出口，否决散点常量。
- **F3 在 TUI 状态行展示生效值**：状态行明示「不含 provider」（`tui.rs:4049`），且 TUI 有既有
  布局契约；改为 CLI 出口（可脚本化、可断言），TUI 面单列后续，否决本期改 TUI。

## 落地与证据

- 红相：`017bc2ec`（10 用例；P1 装载侧 `left: []` / P2 角色序列 / F1 `5≠3`、`1≠2`；
  F2/F3 符号锚点为编译红，另用一次性 stub 验证断言可满足，stub 已删除）。
- 绿相：`pa174` 10/10；`pa173` 8、`pa172` failover 11 / budget 6 / config 8、
  `session_read_from_log` 8、`pa171_error_copy` 89；`--lib` 1405 —— 全 exit 0。
- 门禁：`cargo check -p pony-agent-core --all-targets` 0；`audit-diff` 0；`version:check` PASSED。
- 现场证据留档：`.dev-team/pa174-f4-evidence.txt`（125 条请求 / 75 条带标记 / 单条最多 12 条）。
- 已知遗留（未在本卡修）：TUI 状态行不展示 provider 生效值；重建仅在装载时进行（无后台自愈）。

## 审查整改（`5dc1053f`）

三路对抗审查在 `e927dc39` 报 2 BLOCKING + 多条 ADVISORY，逐条核验后按此收口：

- **B1（BLOCKING，两路独立命中）**：重建丢弃 `branch_id` ⇒ 回退/fork 会话被重建成「全分支并集」（被放弃轮次复活），
  且 `refresh_session_metadata` 会把并集写回索引。修法：装载侧从 `historyCursor.activeBranchId` +
  `historyBranches`/`historyNodes` 取活动血缘，消息折叠复用既有 `projection::fold_all_with_branches`；
  血缘元数据缺失或 activeBranch 不在表中 ⇒ **不重建**并落 `history_rebuild_skipped`（reason=…），绝不取并集。
- **B2（BLOCKING）**：聚合预算耗尽返回合成 `failover budget exhausted` ⇒ 覆盖最后一次真实 provider 错误、
  且不可重试/不满足 `failover_eligible` ⇒ 配了候选链也永不换手。修法：逐次记录真实错误，
  耗尽时返回该真实错误（仅「零次 HTTP」才用合成文案），保持既有分类仍可换手。
- **A2**：stream→sync 回退路径无聚合 deadline ⇒ 按同一 `with_aggregate_sync_deadline` 接入（流式 attempt 预算不变）。
- **A3**：`config effective` 增 `aggregate_budget_ms` / `per_attempt_effective_timeout_ms`，避免
  「合法 300000 配置看起来无上限」的自相矛盾。

**已知证据强度限制（如实登记）**：`b2_aggregate_exhaustion_keeps_real_error_and_allows_failover`
在当前架构下**无判别力**——聚合窗口（~100ms 量级）会被 attempt 上限吸收，耗尽分支无法稳定复现，
故它只作「不得回归」的守门断言，不宣称红相。其余 12 条断言均有行为红相或编译红 + stub 可满足性反证。

## 终审后的遗留清单（已登记，未在本卡修）

- `last_provider_error` 槽位**未按调用清零**：同一 builder 复用（共享 Arc）时，若某次调用零次 HTTP
  尝试（如 `request_timeout_ms: 0`）会返回**上一次调用**的真实错误而非合成文案（审查给的反例）。
- 无候选链时终态错误改为返回**未截断**的 provider 正文（原先经 `preview_text(240/400)` 截断）⇒
  极端情况下日志/调用方载荷显著变大（审查给出 1MiB body 反例）。
- `history.rs` 对**空文本**消息（附件-only 用户轮、纯工具调用 assistant 轮）静默丢弃，无日志/指标。
- 重建在读 `turn_events` 失败时降级为「保持空历史」，只留散文式 eprintln，无结构化事件/trace_id
  （同期既有约定见 `sqlite_session.rs:584-596` 的 `capture_governance_fault`）。
- `refresh_session_metadata` 会把重建结果写回索引 ⇒ 分支血缘修复前的错误并集若已落盘需另行清理
  （本卡只保证此后不再产生）。
- Anthropic 1:1 臂补齐重试后继承 20s+40s 限流长退避（60s）且无本地兜底，而 OpenAI 两臂返回
  `Ok(local fallback)`；arms 在 429 场景下**结果不对称**（无冻结断言约束 Anthropic 1:1 调用次数）。
- 启动期成本：每个「折叠历史为空」的会话一次全量日志解码，142 会话 × 单会话 13098 行 ≈ 1.86M 行
  反序列化（有界但无批量/上限）。
