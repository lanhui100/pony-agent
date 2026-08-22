# Design

## Decision Summary

1. **run_turn 事件化**：`run_turn_inner` 内部经 `emit_event(&NoopTurnEventSink, ...)` 发射完整事件序列（turn:started → user/message → assistant/message → provider/usage×N → turn/end），事件持久化走全局 `current_event_persist()` 通道（与 sink 无关，NoopSink 只是不做前端推送）。终态复用 `build_provider_usage_event` / `build_turn_end_event`（PA-094 已实现三终态发射）。
2. **append_turn 投影化**：签名不变（89 处调用方无感），内部改为"从本 turn 已发射的 UserMessage/AssistantMessage 事件折叠物化 history 追加项"，消息文本以事件为源；持久化走增量 `AppendMessage` command，blob 整包写降频为元数据变更时。
3. **对拍测试 + 豁免清单**：固定场景矩阵（无工具/单工具/多 hop/failed/cancelled/squash）+ 字段集断言；豁免清单显式化为常量表，每项附理由。
4. **StepStart/StepEnd 闭环**：`turn:trace`(phase=calling_model) → StepStart；ProviderUsage 同点发射 StepEnd；AssistantChunk 携带逻辑 hop step（`TurnStreamEvent` 新增 `step: Option<u32>`，运行时 followup 循环传入）。
5. **schema_version 落地**：store_metadata 写 `events.schema_version`；`load_turn_events` 读取校验；解析失败按 ignorable 清单分流（当前清单为空 → 全部 fail loud：会话标记 degraded 并上抛错误，不静默跳过）。
6. **cursor_version 退役**：wire 字段名保留，值来源切换为 `event_watermark`（水位即版本）；四类 history-control command 的冲突检测改水位比较；前端零改动。
7. **trace cache ref-only**：live 写入剥离 observation payload（字段置 None）；`flush_events_tx` 外置 ContextObservation 时**同事务**回填对应 trace 行的 `build_context_observation_ref`。

## Chosen Direction

### 1. run_turn 事件化

- `run_turn_inner` 在 prepare_turn 成功后发射 `turn:started`（payload.text = prepared.user_message，复用现有 UserMessage/ContextObservation 额外发射逻辑）；模型调用完成/工具 followup 各 hop 结算处发射 provider/usage（复用 per-call records 遍历）；终态发射 assistant/message + turn/end（completed）或 turn/end（failed/cancelled）。
- **早退路径发射责任（审核 P0 修订）**：不变式"每个 turn:started 必有配对 turn:end"。逐点枚举：
  - `prepare_turn` 失败：turn:started 之前早退 → 不发任何事件（与 streaming 入口对齐：streaming 同样在 prepare 失败时只发 turn:failed 无 started——两入口口径一致，等价性测试覆盖此分支）。
  - hook fail-turn / plan 失败 / provider 失败（turn:started 之后）：必须发射 `turn/end`（reason=Error）再返回，禁止悬挂 turn。
- 与 streaming 入口的差异仅两点：无 chunk 流（AssistantMessage 直接落）、无 sink 推送。事件序列等价性有专项测试（逐事件类型对比两入口，**含 failed/cancelled 分支**）。
- graph sync fallback（control_plane:1857）与 Tauri command、non_tauri_harness 自动受益（同一 runtime 入口）。

### 2. append_turn 投影化

- 现状：`append_turn(session_id, user_message, assistant_message, ...)` push 消息到 `session.history` + `save_to_backend()` 整包写。
- 改造后：
  - 消息来源：本 turn 的 `UserMessage` / `AssistantMessage` 事件。**取数时序（审核 P0 修订）**：事件 flush 原子序保证——persist 闭包中事件表事务提交先于内存缓冲清空（当前实现为同一锁内 remove→flush，需显式断言该顺序并加测试）；物化读取按 `(session_id, turn_id)` 定位事件表（缓冲已清、表已可见），不存在"两处皆空"窗口。
  - 兜底：仅当 backend 无事件支持（MemoryBackend 等，`load_turn_events` 返回空且能力标志为否）时回退调用方参数直推；事件能力存在但取数失败 → 记录日志 + 上抛异常（可观测），禁止静默回退（防止投影化被架空）。
  - 部分事件（failed turn 只有 user/message 无 assistant/message）：物化出 user 条目，assistant 条目按 failed 语义补占位（与 streaming 入口的 append_failed_turn 行为对齐）。
  - 持久化：history 追加走 `PersistCommand::AppendMessage` 增量写（normalized_messages 表已存在）；blob 整包写仅在会话元数据（title/summary/turn_count）变更时执行。
- 写放大消除证据（审核修订：弃用绝对字节数）：主指标 = persist-command 拦截的结构性断言（message-only turn 的整包 store 写次数 = 0、AppendMessage = N）；辅指标 = `wal_autocheckpoint=0` + TRUNCATE checkpoint 后 db+wal 文件尺寸比例阈值（改造后 ≤ 基线 50%）；基线数字固化进任务完成记录存档。

### 3. 对拍测试与豁免清单

- 场景矩阵：无工具基线 / 单工具 / 多 hop（≥2 provider call）/ failed 中途 / cancelled / history squash / 分支 fork 后 checkout。
- 断言口径：约定字段集 = timeline 条目（kind/label/state/sequence/text/tool_activities/token 指标）+ trace 记录级（phase/provider/token/duration/event_type）。
- 豁免清单（显式常量，每项附理由）：
  - `updated_at` / `event_id` / `emitted_at_ms`（时钟语义）
  - `title` / `session_id`（运行时装饰，投影不自洽——由 annotate 补写）
  - `prepare_retrieval` 条目（需 payload 判断 build_context_uses_retrieval，事件只有引用）
  - build_context 条目的 provider 六元数据（事件无承载，后续可扩展）
  - failed/cancelled 末 hop state=error/cancelled vs 折叠 completed（待 #4 StepEnd 落地后收敛，先豁免并登记）

### 4. StepStart/StepEnd 闭环

- 映射扩展（turn_flow.rs `build_turn_event`）：
  - `turn:trace` + phase=calling_model → `StepStart { step, first_token_latency_ms: None }`（canonical event_type 已是 turn.model_call_started）
  - ProviderUsage 发射点同步发射 `StepEnd { step }`
- chunk step：`TurnStreamEvent` 新增 `step: Option<u32>`（serde default，wire 向后兼容）；运行时 followup 循环以 hop 索引填充；`build_turn_event` 的 turn:delta 用 `payload.step.unwrap_or(0)`。
- TraceProjection：AssistantChunk 按 (turn_id, step) 定位 call_model 条目（ensure_call_model_entry 已按 step 索引，天然支持多 hop）；settle_timeline 的兜底逻辑保留（legacy 事件流无 StepStart 时）。

### 5. schema_version 契约

- 写入：`ensure_schema` 时 store_metadata 写 `events.schema_version = EVENT_SCHEMA_VERSION`；版本升级时 bump 常量 + 迁移分支。
- **缺失 key 语义（审核 P1 修订）**：store_metadata 无该 key（本 change 之前创建的所有库与测试 fixture）视为 version 1（当前版本）并在首次写入时回填——存量数据保持可读，避免大面积回归。
- 校验：`load_turn_events` 前置读取；不匹配时返回明确错误（数据不可用，而非部分视图）。
- fail loud：事件 payload 解析失败 → 该会话标记 `event_stream_degraded`（SessionState 新增标记）+ 错误经宿主 load API 上抛（UI 可见"事件流损坏"），不静默跳过。ignorable 清单常量预留（当前为空）。

### 6. cursor_version 退役

- `HistoryCursor.cursor_version` 字段保留（wire 兼容），值来源改为 `event_watermark`；写入点统一到 finalize_event_watermark。
- **水位单调性不变式（审核 P0 修订，消除 ABA 窗口）**：checkout/fork/squash 必须发射对应 history-control 事件（checkpoint/checkout、fork/created、history/squash——事件类型已存在），使水位严格递增。回滚操作移动的是投影位置（折叠水位回退到节点区间），但事件日志水位本身只增不减；乐观锁比较的是日志水位，任何交错变更（含回滚）都严格推进版本 → 无 ABA。
- 四类 history-control command 的 `expected_cursor_version` 冲突检测：比较水位而非独立计数器；stale 判定语义在单调域内与原计数器一致。
- 前端 `cursorVersion` 引用（sessions.ts 3 处）零改动（字段名与语义"乐观锁版本"一致，只是值的来源变了）。
- ADR 口径达成："seq 水位本身就是版本"。

### 7. trace cache ref-only

- live 写入：`record_turn_trace_in_memory` 构造 TurnTraceRecord 时 `build_context_observation` 置 None（payload 不入缓存/blob）。
- ref 回填：`flush_events_tx` 外置 ContextObservation 时（已有同事务写 bco 表逻辑），同事务 UPDATE 对应 trace 行的 observation ref（extension_json 或新列）。
- legacy 兼容：既有内嵌数据读取路径不变；PA-094 的前端按需加载已消费 ref。
- 测试：live turn 完成后 trace 行 raw_json 不含 requestFormat；ref 可加载且与原始一致。

## Verification Strategy

- 单元：run_turn 事件序列逐类型断言；StepStart/StepEnd 映射；chunk step 透传；schema_version 校验矩阵（匹配/不匹配/坏 payload）；cursor 水位冲突检测。
- 对拍：场景矩阵 × 约定字段集，豁免清单外零差异。
- 集成：Tauri 同步 command → 清 trace 表 → 重启重建一致；多 hop turn 重建 call_model 条目数 = hop 数。
- 性能：append_turn 改造前后 SQLite 写入字节数对比（写放大消除证据）。
- 回归：core + src-tauri + 前端全量全绿。
