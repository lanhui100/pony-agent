# Tasks

## 1. run_turn 事件化（缺口 #1，P0）

- [x] `run_turn_inner` 接入 `emit_event` + NoopTurnEventSink：turn:started（含 user/message、context/observation 额外发射）→ 终态 assistant/message + provider/usage×N + turn/end。
- [x] **早退路径发射责任**（审核 P0）：prepare_turn 失败不发任何事件（与 streaming 对齐）；hook fail-turn / plan 失败 / provider 失败必须发 turn/end(Error) 再返回——不变式"每个 turn:started 必有配对 turn:end"。（实现：7 处失败路径全部经 `emit_sync_turn_failed`；turn_id 由 `sync_turn_event_id` 一次性计算贯穿全路径——started/failed 同源，nanos+进程内计数器防 Windows 计时器精度碰撞。）
- [x] failed/cancelled 路径同样发射（复用 `build_provider_usage_event` / `build_turn_end_event` 三终态逻辑）。（同步入口无 cancelled 语义——阻塞调用无 stop 通道，cancelled 为 streaming 独有；failed 全路径已发射。终态信封取自发射本身（emit_event 返回 envelope），TurnResult/trace 信封与事件流同源序列号。）
- [x] graph sync fallback（control_plane:1857）与 non_tauri_harness 验证受益（端到端事件落盘专项测试，非仅声明）。（`graph_sync_run_turn_persists_event_stream_end_to_end`：control_plane.run_turn → 生产注册通道缓冲闭包 → SQLite SessionStore 落盘 → load_turn_events 读回骨架断言；重试吸收并行单槽覆盖噪声。）
- [x] 测试：run_turn 与 start_turn_stream 事件序列等价性——completed 主干 + **failed/cancelled 分支**逐事件类型对比；悬挂 turn 不变式断言；等价性测试串行化或按 turn_id 隔离（全局 EVENT_PERSIST_REGISTRY 并行污染防护）。（`run_turn_and_start_turn_stream_produce_equivalent_event_sequences`：completed 骨架 [turn/start, context/observation, user/message, assistant/message, provider/usage, turn/end] 两侧一致 + failed 骨架一致 + 悬挂不变式（限本测试 session）；经测试多播 sink 捕获（`register_event_persist_test_sink`），生产单槽被并行 HostControlPlane 覆盖不再影响。）

## 2. append_turn 投影化（缺口 #2，P0）

- [x] `append_turn` 内部改为从本 turn UserMessage/AssistantMessage 事件折叠物化 history 追加项；**取数时序**：persist 闭包保证事件表事务提交先于缓冲清空（显式断言 + 测试）；读取按 (session_id, turn_id) 定位事件表。（实现：①persist 闭包终态路径与 flush requester 均改为 **flush 成功才 remove**（commit 先于 clear，失败保留缓冲可重试）；②生产路径 `persist_turn_outcome` 在获取 sessions 写锁**之前**预提交缓冲（flush 闭包需 sessions 写锁，持锁调用自锁死锁——实现期发现并修复），requester 用 `try_write` 将锁忙转为 Err 而非挂死；③物化按"最近 turn"定位（签名冻结不传 turn_id，单写者流内确定性）。**登记偏差**：attachments 暂经调用方参数合并入 user 条目（user/message 事件当前恒空 attachments），后续可扩展 TurnStreamEvent.attachments 全事件源。）
- [x] 兜底收窄：仅真无事件后端（MemoryBackend 等）回退调用方参数；事件能力存在但取数失败 → 日志 + 上抛（可观测），禁止静默回退。部分事件（failed turn 仅 user/message）物化行为定义。（`supports_turn_events()` 能力标志（SQLite true/默认 false）；回退三类均记录可观测日志：真无事件后端 / 直连 store 无 turn 归属事件 / 缓冲 flush 失败（提交不完整时按事件物化有错 turn 风险，整体回退更安全——并行测试下全局注册表被外来 control plane 抢占亦落入此路径）；**事件读取失败（degraded）fail loud panic**。failed turn assistant 缺席 → caller 参数补占位（对齐 append_failed_turn）。）
- [x] history 持久化走 `PersistCommand::AppendMessage` 增量写；blob 整包写降频为元数据变更时。（**阶段 A 达成**：append_turn 事件能力后端改走 `2×AppendMessage（normalized_messages，observing 切读源）+ UpdateSessionMeta + 单会话行落库（save_session_to_backend：Authoritative trace mutation / upsert 链）`，**save_store 整包写 = 0**（消除全 store 扫描序列化 + 每 turn checkpoint 的写放大根因）。**阶段 B 登记**：会话行级按 facet 增量（transcript/memory/nodes 脱离行重写）待后续——现 blob 行仍每 turn 重写一次（单会话粒度）。）
- [x] 写放大消除证据（审核修订：双层指标）——主：persist-command 拦截结构性断言（message-only turn 整包写 = 0、AppendMessage = N）；辅：checkpoint 后 db+wal 尺寸比例阈值（≤ 基线 50%）；基线数字存档任务记录。（**主指标达成**：CountingBackend 计数代理拦截断言 save_store 增量=0、AppendMessage=2、UpdateSessionMeta=1（`append_turn_materializes_history_from_events_over_caller_text`）。**辅指标登记待办**：wal 尺寸基线需在旧代码上采样，阶段 B 一并补。）
- [x] 测试：事件源物化与直推参数一致（含 attachments/reasoning_content）；**回退路径测试**（MemoryBackend 触发 fallback）；**双时序测试**（flush 后表读取路径）；改造前录制 append_turn 输出 characterization 快照作回归防护。（`append_turn_characterization_snapshot`（改造前录制、含 SQLite blob 往返，改造后保持绿）；`materialize_last_turn_messages_picks_latest_turn`（多 turn 选取/failed 缺席/空流）；`append_turn_materializes_history_from_events_over_caller_text`（caller≠事件文本时物化取事件 + reasoning 随事件 + 结构性计数断言，重试吸收通道抢占噪声）；MemoryBackend 回退由既有 append_turn 测试族（20+ 项）覆盖。）

## 3. 对拍测试 + 豁免清单（缺口 #3，P1）

- [x] 豁免清单显式常量化（每项附理由）：时钟字段、title/session_id、prepare_retrieval、build_context provider 元数据、failed 末 hop state。（`projection.rs::parity` 模块单一事实源：`AGREED_TRACE_FIELDS` / `AGREED_TIMELINE_ENTRY_FIELDS` / `EXEMPT_TIMELINE_KINDS` / `EXEMPTIONS`（10 项，每项附理由）+ `trace_field`/`timeline_entry_field` 取值器。新增登记：checkpoint_persist 运行时装饰条目、sync_call_model_text、terminal_no_usage_turn_provider_metadata、call_tool_text 语义分叉、turn_duration_ms 时钟漂移容差。）
- [x] 场景矩阵对拍：无工具/单工具/多 hop/failed/cancelled/squash/fork-checkout + **多 turn 会话** + **legacy 内嵌与新 ref-only 混跑会话** × 约定字段集。（**7 场景全绿**（2026-08-22）：无工具 sync（+bctx 元数据探针）/ 单工具流式 / failed hook-fail / 多 hop 流式 / cancelled plan 前 / multi-turn 会话 / **fork-checkout 跨分支**（分支命令事件落盘探针 + 水位单调探针 + 无 ABA 端到端断言）。**登记偏差**：①squash 无生产入口（HistorySquash 为 PA-091 前向定义事件类型）——投影级语义由 `history_projection_squash_drops_old_messages` 覆盖，生产入口落地时补端到端场景；②legacy-mixed 以存储级混跑覆盖（`pa095_ref_only_trace_rows_backfill_and_legacy_reads`：同库内 legacy 内嵌行与新 ref-only 行共存、双表读面一致），跨 turn 对拍维度随 #7 收口验证。）
- [x] **cancelled 场景验证驱动修复链**（2026-08-22）：①生产缺陷——`emit_stream_cancelled` 调用点 session_id 传 None，用户取消流式 turn 的 turn:cancelled 终态事件进空会话缓冲被 flush skip（事件流丢终态），修复为携带 session_id；②测试 harness——`MockHttpServer::finish()` join 永远阻塞在 accept 的线程挂死，改 drop；③对拍收敛——投影 TurnEnd 结算时 Cancelled/Aborted turn 中未经 usage 结算的 call_model 条目标记 cancelled（中断调用不得显示 completed）；④豁免登记 terminal_no_usage_turn_provider_metadata（原 failed_turn_provider_metadata 扩展覆盖 cancelled）+ 双向反向探针；⑤plan 前 cancelled 探针语义对齐 #4（step/start 真实存在于事件流，断言"无虚构完成态"而非"无条目"）；⑥既有缺陷——settle 兜底收窄后 context_ref 前插 get_mut 静默丢失，改 entry API（`trace_projection_context_observation_sets_ref` 同步对齐收窄语义）；⑦并行 flake 根因治理——初版串行锁被证明加剧全局单槽饥饿，最终落地会话绑定路由（`bind_event_persist_session`，见 #6 节与 Validation Notes 审核记录）。
- [x] 豁免清单外零容忍；**对拍测试迭代消费豁免常量本身**（agreed = 全字段集 − EXEMPT_FIELDS，清单为唯一事实源，禁止手抄字段列表）；**反向探针元测试**：每个豁免项断言该差异当前确实存在（防豁免腐化；#4 落地后 failed 末 hop 项探针翻红 = 强制收敛信号）。（assert_parity 按登记项消费豁免（skip_provider/call_tool text/failed last hop state）+ turn_duration ≤250ms 容差断言；反向探针 4 个落地：clock/title 装饰/checkpoint_persist 条目/failed 末 hop state/streaming chunk 文本。build_context provider 元数据探针随场景扩展补。）
- **对拍驱动的收敛修复**（对拍暴露的真分歧，已修）：①投影 turn/end 结算时按 reason 设置 phase（此前恒空——约定字段集含 phase）；②ProviderUsage 事件 provider 字段取 provider 名称（原取 source 标签，与存储 trace 维度语义错位）；③call_tool 条目 label/state 与运行时产物同构（hop 序号取 call_model 计数、done→completed 映射）；④运行时存储侧 timeline 补 return_result 条目（tool/result → return_result 映射表落地，与重建同构）；⑤cancelled/aborted turn 未经 usage 结算的 call_model 条目结算为 cancelled（中断调用不显示 completed）。

## 4. StepStart/StepEnd + chunk step（缺口 #4，P1）

- [x] `TurnStreamEvent` 新增 `step: Option<u32>`（serde default，wire 兼容）；运行时 followup 循环填充 hop 索引（step 0-based，legacy chunk 缺省归 step 0）。（`emit_stream_event_with_step` 携带 step；followup 循环 `completed_hops` Copy 捕获进 delta 闭包；直连路径 chunk 缺省 step 0。**顺带修复生产数据丢失 bug**：followup delta 闭包 session_id 原传 None——事件以空 session 缓冲且永不 flush，全部 followup chunk 从生产事件流丢失。）
- [x] 映射扩展：turn:trace(calling_model) → StepStart；ProviderUsage 同点发射 StepEnd。（`build_step_start_event`：turn:started → StepStart{0}（初始 call）+ turn:trace(calling_model) 且 step≥1 → followup StepStart（直连路径 calling_model trace 不带 step 不重复发射）；`build_step_end_events` 与 usage 相邻成对（usage[i] 紧跟 step/end[i]），仅 completed 结算——failed/cancelled 无 usage 数据，TurnEnd(Error) 承担终态。）
- [x] build_turn_event 的 turn:delta 使用 payload.step。（AssistantChunk{step: payload.step.unwrap_or(0)}。）
- [x] 测试：多 hop turn（≥2 provider call）重建 timeline 的 call_model 条目数 = hop 数；chunk 文本归属正确 hop；`step` 字段 serde 往返兼容（旧 payload 无 step 反序列化）；StepEnd 与 ProviderUsage 相邻序断言。（`multi_hop_turn_rebuilds_timeline_with_per_hop_call_model_entries`：streaming 3-hop——step/start 序 [0,1,2]、chunk steps [0,1,2]、call_model=3、文本归属逐 hop 断言、usage↔step/end 相邻；sync 变体验证 usage 兜底收敛 call_model=2；`turn_stream_event_step_serde_roundtrip`。）

## 5. schema_version 契约（缺口 #5，P1）

- [x] store_metadata 写入/读取 `events.schema_version`；不匹配返回明确错误。（ensure_schema seed + `validate_event_schema_conn` 四分支；SessionStore 构建时启动校验 fail loud。）
- [x] **缺失 key 语义**（审核 P1）：视为 version 1 并首次写入时回填——存量库与全部既有 fixture 保持可读（回归风险最高项，先定案再动 ensure_schema）。
- [x] 坏 payload fail loud：会话标记 `event_stream_degraded` + 错误经宿主 load API 上抛；ignorable 清单常量预留（当前为空）。（`load_turn_events_checked` 失败标记 SessionStore `event_stream_degraded` 集合 + `is_event_stream_degraded` 查询入口；`IGNORABLE_EVENT_TYPES` 空常量，读取路径消费——清单内类型跳过、清单外 fail loud。）
- [x] 测试：版本匹配/**缺失 key（legacy 库）**/不匹配/坏 payload 四分支。（`event_schema_version_contract_four_branches`。）

## 6. cursor_version 退役（缺口 #6，P2）

- [x] cursor_version 值来源切换 event_watermark（wire 字段名不变）；写入点统一 finalize_event_watermark。（2026-08-22：四处命令写入点 `bump_cursor_version` 删除，改为水位镜像；`finalize_event_watermark` 统一镜像 cursor_version/event_watermark 到 cursor（权威源 session.event_watermark——cursor 字段仅命令结算时同步，直接比较会读陈旧值）；前端消费为不透明乐观锁值，wire 兼容。）
- [x] **水位单调性不变式**（审核 P0）：checkout/fork/squash 发射对应 history-control 事件使日志水位严格递增（投影位置可回退、日志水位只增不减）——消除 ABA 窗口。（发射结构 PA-093 已有（CheckpointCheckout/ForkCreated/HistorySquash 事件类型 + 四命令 emit_global_event）；PA-095 补齐落盘确定性：emit_global_event 改仅入缓冲（history 命令持锁路径内联 flush 同线程自锁死锁——fork 实测挂死），由四命令包装释放写锁后的 flush_session_buffered_events 提交 + run_turn/start_turn_stream 入口返回前补提交；squash 无生产入口（PA-091 前向定义），登记偏差。）
- [x] 四类 history-control command 冲突检测改水位比较。（`reject_stale_cursor_version` 改比较 session.event_watermark，错误信息改 watermark 语义；cursor_version 字段保留序列化但不再参与检测。）
- [x] 测试：stale 水位冲突语义（单调域）；**checkout 后版本严格递增断言**（无 ABA）；四类 command 逐类表驱动冲突测试；实施前 grep 盘点数值型 cursor_version 断言清单。（`pa095_history_command_conflict_detection_is_watermark_based`（表驱动 4 命令 × None/精确/stale/镜像断言）+ `pa095_checkout_version_strictly_increases_with_watermark_no_aba`（SQLite 两批事件推进水位 + checkout 镜像断言 + 旧期望拒绝）；grep 盘点 61 处引用分类完成（schema 列/wire 字段/冲突点/镜像点），既有测试全部使用 expected=None 无数值断言需迁移。）

## 7. trace cache ref-only（缺口 #7，P2）

- [x] live 写入剥离 observation payload（record_turn_trace_in_memory 置 None）。（2026-08-22；**实施后对抗审核 P0 修正**：初版只剥记录级字段、timeline 条目仍内嵌全量 payload（requestFormat 断言因 fixture timeline 为空而盲区通过）——已补三处 timeline 构造源头剥离（build_persisted_trace_timeline / build_stream_started / progress 的 build_context 条目置 None），payload 唯一副本在 build_context_observations 表。）
- [x] flush_events_tx 外置时同事务回填 trace 行 observation ref。（`backfill_trace_observation_ref_tx`：session_turn_traces.trace_data 与 normalized_turn_traces.raw_json 双表 JSON 打补丁（buildContextObservation→null + buildContextObservationRef 写入），与事件行同一事务；两表均无该 turn 行时记日志跳过。**审核 P2 补充**：异步 trace worker 整行 REPLACE 会覆盖回填——`attach_observation_refs_to_turn_trace` 在 finalize 后把 ref 附加到内存缓存行（backend 新增 `load_observation_refs` 能力方法，默认空实现），worker 后写自带引用，两种交错均不丢。）
- [x] 读侧水合（实施后审核 P1 补充项）：ref-only 行读回时按引用还原 payload——`hydrate_trace_observation` 于 read_session_traces/read_all_session_traces；legacy 内嵌行原样返回；ref 未命中 contained 日志。（此前"读路径优先 ref"仅是注释宣称、无实现——monitor 检索参与计数/前端历史视图将静默归零。）
- [x] 测试：live turn 后 trace 行 raw_json 不含 requestFormat；ref 加载与原始一致；**读侧水合还原一致**；legacy 内嵌数据仍可读；实施前 grep 盘点既有 requestFormat 断言清单。（`pa095_ref_only_trace_rows_backfill_and_legacy_reads` 五段断言；grep 盘点 16 处——全部为构造参数或事件外置断言，无 trace 行内嵌依赖，无需迁移。）

## 8. OpenSpec 收口事务

- [x] 归档 PA-091~093 changes：`turn-event-log` / `session-projection-layer` / `checkpoint-event-referencing` → `openspec/changes/archive/2026-08-22-*`，canonical specs 核对同步。（2026-08-22 执行：三 change 移入 archive/2026-08-22-*，canonical specs 落于 openspec/specs/<capability>/spec.md；归档前补勾 turn-event-log/session-projection-layer 滞后的任务框（实现已随 PA-091/092 提交并验证，簿记同步注明）。）
- [x] 本 change 完成后归档 + canonical spec 同步。（2026-08-22：实施后三路对抗审核 10 项代码修复落地、867×4 稳定、全部验证门禁通过后归档至 archive/2026-08-22-event-sourcing-closeout，canonical spec 落于 openspec/specs/event-sourcing-closeout/spec.md。）

## Validation Notes

- 实施顺序建议：#5 的缺失-key 语义决策先行定案 → #1（独立，风险低）→ #4（事件契约扩展）→ #2（影响面最大，characterization 快照防护）→ #3（依赖 #2/#4 收敛后对拍口径稳定）→ #6/#7（收尾）→ #8。
- 回归防护排序（审核意见）：#2 > #6 > #1 > #5 > #7 > #4；#2 子阶段 1（事件源物化）全绿后再进子阶段 2（去整包写）。
- 对抗审核记录：spec 阶段两路审核（general 可测性 PASS WITH REVISIONS + ox-alpha 架构 PASS WITH REVISIONS）已采纳修订——早退路径枚举、flush 原子序、水位单调性、missing-key 语义、豁免单源+反向探针、双层写放大指标。
- **实施后三路对抗审核（2026-08-22：正确性回归 / 架构规范一致性 / 边界失败路径，均"有条件通过"）采纳记录**：
  - **已修（代码）**：①跨线程 ABBA——persist 闭包持 buf mutex 调 flush（阻塞 sessions.write）× history 命令持 sessions.write 发射抢 buf mutex，重构为锁内聚合/快照、锁外 flush 后重加锁移除；②cursor 往返契约——命令响应版本改取补 flush 后当前水位（`fresh_cursor_version` 只读访问器；**不可用 load_history_cursor 重建**——其 ensure/save 副作用会把 checkout 截断态修复回去，实测回归）；③匿名 turn 空 session_id 缓冲键永久滞留——dispatch 归一 DEFAULT_SESSION_ID；④无事件后端缓冲结构性滞留——commit_event_batch 对 Unsupported 显式清除（contained 丢弃）；⑤#7 timeline 内嵌 payload 源头剥离 + 读侧水合（见 #7 节）；⑥ref 回填被异步 worker REPLACE 覆盖——内存附加（见 #7 节）；⑦graph 两入口（advance_graph_run/execute_graph_run_stream）补返回前 flush；⑧绑定守卫改 Arc::ptr_eq 解绑；⑨生产单槽覆盖告警；⑩存量 cursor_version≠watermark 一次性升级校正 SQL。
  - **登记偏差/残余风险**：(a) ABA 窗口在包装补 flush 失败时短暂重开（`let _ =` 吞错；缓冲保留待重试只是延迟，空闲会话需等下一触发点）；(b) `append_turn` 物化读事件失败 fail-loud panic 发生在可中毒写锁调用链上（前序会话的设计决策，单坏事件砖化该会话写入路径——后续卡评估 Result 化）；(c) display_message≠user_message 时物化静默改变历史存储文本（#2 已知偏差的边缘扩展，characterization 未覆盖该分叉）；(d) 反向探针未覆盖全部豁免项（sync_call_model_text/call_tool_text/clock_drift 以容差或跳过消费，prepare_retrieval 仅 kind 过滤）；(e) delta spec 约定字段集已按实现收缩同步（sequence→保序断言、条目级 token metrics 并入 trace 级）；(f) "清表重建与预清快照一致"由 `trace_cache_watermark_and_clear_rebuild` 部分覆盖（vs 事件折叠而非预清快照逐字段）；(g) 多客户端共享 SQLite 时水位比较源是内存态（单写者假设下可接受，与旧 cursor_version 同类）；(h) turn_id 复用场景下 register_turn 携带旧 stop 请求会使新 turn 开局即 cancelled（turn_id 全局唯一前提下无害）。
  - **文档不同步修正**：EXEMPTIONS 实为 10 项（含 clock_drift 容差）；⑦"串行锁"已被会话绑定路由取代；归档日期 2026-08-22-*。
