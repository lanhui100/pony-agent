# Tasks

## 1. run_turn 事件化（缺口 #1，P0）

- [ ] `run_turn_inner` 接入 `emit_event` + NoopTurnEventSink：turn:started（含 user/message、context/observation 额外发射）→ 终态 assistant/message + provider/usage×N + turn/end。
- [ ] **早退路径发射责任**（审核 P0）：prepare_turn 失败不发任何事件（与 streaming 对齐）；hook fail-turn / plan 失败 / provider 失败必须发 turn/end(Error) 再返回——不变式"每个 turn:started 必有配对 turn:end"。
- [ ] failed/cancelled 路径同样发射（复用 `build_provider_usage_event` / `build_turn_end_event` 三终态逻辑）。
- [ ] graph sync fallback（control_plane:1857）与 non_tauri_harness 验证受益（端到端事件落盘专项测试，非仅声明）。
- [ ] 测试：run_turn 与 start_turn_stream 事件序列等价性——completed 主干 + **failed/cancelled 分支**逐事件类型对比；悬挂 turn 不变式断言；等价性测试串行化或按 turn_id 隔离（全局 EVENT_PERSIST_REGISTRY 并行污染防护）。

## 2. append_turn 投影化（缺口 #2，P0）

- [ ] `append_turn` 内部改为从本 turn UserMessage/AssistantMessage 事件折叠物化 history 追加项；**取数时序**：persist 闭包保证事件表事务提交先于缓冲清空（显式断言 + 测试）；读取按 (session_id, turn_id) 定位事件表。
- [ ] 兜底收窄：仅真无事件后端（MemoryBackend 等）回退调用方参数；事件能力存在但取数失败 → 日志 + 上抛（可观测），禁止静默回退。部分事件（failed turn 仅 user/message）物化行为定义。
- [ ] history 持久化走 `PersistCommand::AppendMessage` 增量写；blob 整包写降频为元数据变更时。
- [ ] 写放大消除证据（审核修订：双层指标）——主：persist-command 拦截结构性断言（message-only turn 整包写 = 0、AppendMessage = N）；辅：checkpoint 后 db+wal 尺寸比例阈值（≤ 基线 50%）；基线数字存档任务记录。
- [ ] 测试：事件源物化与直推参数一致（含 attachments/reasoning_content）；**回退路径测试**（MemoryBackend 触发 fallback）；**双时序测试**（flush 后表读取路径）；改造前录制 append_turn 输出 characterization 快照作回归防护。

## 3. 对拍测试 + 豁免清单（缺口 #3，P1）

- [ ] 豁免清单显式常量化（每项附理由）：时钟字段、title/session_id、prepare_retrieval、build_context provider 元数据、failed 末 hop state。
- [ ] 场景矩阵对拍：无工具/单工具/多 hop/failed/cancelled/squash/fork-checkout + **多 turn 会话** + **legacy 内嵌与新 ref-only 混跑会话** × 约定字段集。
- [ ] 豁免清单外零容忍；**对拍测试迭代消费豁免常量本身**（agreed = 全字段集 − EXEMPT_FIELDS，清单为唯一事实源，禁止手抄字段列表）；**反向探针元测试**：每个豁免项断言该差异当前确实存在（防豁免腐化；#4 落地后 failed 末 hop 项探针翻红 = 强制收敛信号）。

## 4. StepStart/StepEnd + chunk step（缺口 #4，P1）

- [ ] `TurnStreamEvent` 新增 `step: Option<u32>`（serde default，wire 兼容）；运行时 followup 循环填充 hop 索引（step 0-based，legacy chunk 缺省归 step 0）。
- [ ] 映射扩展：turn:trace(calling_model) → StepStart；ProviderUsage 同点发射 StepEnd。
- [ ] build_turn_event 的 turn:delta 使用 payload.step。
- [ ] 测试：多 hop turn（≥2 provider call）重建 timeline 的 call_model 条目数 = hop 数；chunk 文本归属正确 hop；`step` 字段 serde 往返兼容（旧 payload 无 step 反序列化）；StepEnd 与 ProviderUsage 相邻序断言。

## 5. schema_version 契约（缺口 #5，P1）

- [ ] store_metadata 写入/读取 `events.schema_version`；不匹配返回明确错误。
- [ ] **缺失 key 语义**（审核 P1）：视为 version 1 并首次写入时回填——存量库与全部既有 fixture 保持可读（回归风险最高项，先定案再动 ensure_schema）。
- [ ] 坏 payload fail loud：会话标记 `event_stream_degraded` + 错误经宿主 load API 上抛；ignorable 清单常量预留（当前为空）。
- [ ] 测试：版本匹配/**缺失 key（legacy 库）**/不匹配/坏 payload 四分支。

## 6. cursor_version 退役（缺口 #6，P2）

- [ ] cursor_version 值来源切换 event_watermark（wire 字段名不变）；写入点统一 finalize_event_watermark。
- [ ] **水位单调性不变式**（审核 P0）：checkout/fork/squash 发射对应 history-control 事件使日志水位严格递增（投影位置可回退、日志水位只增不减）——消除 ABA 窗口。
- [ ] 四类 history-control command 冲突检测改水位比较。
- [ ] 测试：stale 水位冲突语义（单调域）；**checkout 后版本严格递增断言**（无 ABA）；四类 command 逐类表驱动冲突测试；实施前 grep 盘点数值型 cursor_version 断言清单。

## 7. trace cache ref-only（缺口 #7，P2）

- [ ] live 写入剥离 observation payload（record_turn_trace_in_memory 置 None）。
- [ ] flush_events_tx 外置时同事务回填 trace 行 observation ref。
- [ ] 测试：live turn 后 trace 行 raw_json 不含 requestFormat；ref 加载与原始一致；legacy 内嵌数据仍可读；实施前 grep 盘点既有 requestFormat 断言清单。

## 8. OpenSpec 收口事务

- [ ] 归档 PA-091~093 changes：`turn-event-log` / `session-projection-layer` / `checkpoint-event-referencing` → `openspec/changes/archive/2026-08-21-*`，canonical specs 核对同步。
- [ ] 本 change 完成后归档 + canonical spec 同步。

## Validation Notes

- 实施顺序建议：#5 的缺失-key 语义决策先行定案 → #1（独立，风险低）→ #4（事件契约扩展）→ #2（影响面最大，characterization 快照防护）→ #3（依赖 #2/#4 收敛后对拍口径稳定）→ #6/#7（收尾）→ #8。
- 回归防护排序（审核意见）：#2 > #6 > #1 > #5 > #7 > #4；#2 子阶段 1（事件源物化）全绿后再进子阶段 2（去整包写）。
- 对抗审核记录：spec 阶段两路审核（general 可测性 PASS WITH REVISIONS + ox-alpha 架构 PASS WITH REVISIONS）已采纳修订——早退路径枚举、flush 原子序、水位单调性、missing-key 语义、豁免单源+反向探针、双层写放大指标。
