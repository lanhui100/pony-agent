# PA-095 事件溯源收尾（ADR 0008 承诺缺口关闭）

## Basic Info

- ID: PA-095
- Status: Done
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-21
- Updated At: 2026-08-22
- OpenSpec Change: `openspec/changes/archive/2026-08-22-event-sourcing-closeout/`（2026-08-22 归档，canonical spec 落于 `openspec/specs/event-sourcing-closeout/spec.md`）
- Spec 状态: spec 两路对抗审核完成并采纳修订（2026-08-21：general 可测性 PASS WITH REVISIONS + ox-alpha 架构 PASS WITH REVISIONS；consultant 两次空结果未参与）

## Background

ADR 0008（`docs/decisions/0008-event-sourcing-evolution.md`）四阶段主体已落地（PA-091~094，三轮对抗审核加固），但经 2026-08-21 对照 ADR 原文逐项审核，确认 **5 项 ADR 明示承诺未落地 + 2 项审核发现的架构级缺口**。这些缺口若不关闭，"四阶段完成"的表述会掩盖承诺偏差，且部分缺口（同步 run_turn 绕过事件流）使事件权威在生产入口上不成立。

## Goal（7 项缺口，按影响排序）

1. **同步 `run_turn` 接入事件流**（P0）：control_plane:1135 / runtime:2507 的同步入口直接写 history/trace、零事件产生——该入口的 turn 无法从事件重建。统一生产入口走同一事件编排。
2. **`append_turn` 改造为事件折叠**（P0，ADR 阶段 2 核心承诺）：当前仍直接 push history + 整包快照，写放大未消除；改为"append 事件 + 增量折叠投影"，快照降级为投影缓存。
3. **blob↔事件折叠对拍测试**（P1，ADR 阶段 2 验收承诺）：从事件重建的快照与 blob 快照逐字段一致；已知结构性分歧（prepare_retrieval 条目、provider 元数据、title/session_id、failed 末 hop state 等）固化为显式豁免清单或修复。
4. **StepStart/StepEnd 发射 + AssistantChunk 逻辑 step**（P1）：多 hop turn 的模型输出全部聚合到首个 call_model（AssistantChunk 固定 step=0），timeline 重建与运行时产物结构性分歧；需 TurnStreamEvent 携带 step + 运行时 hop 追踪 + 每次 provider call 发射 StepStart/StepEnd。
5. **`event_schema_version` 落地**（P1，ADR 阶段 1 承诺）：写入事件 payload/store_metadata、读取校验、未知必需事件 fail loud / ignorable 可跳过（借鉴 dsh SESSION_FORMAT_VERSION）。当前仅定义常量从未使用。
6. **`cursor_version` 乐观锁退役**（P2，ADR 阶段 3 承诺）：seq 水位本身就是版本；session.rs 尚有 23 处引用在用，双版本机制并存。
7. **trace cache 只存 ref（R4b No duplicate storage）**（P2）：live 路径 trace cache 仍内嵌全量 build_context_observation（raw_json + blob 三处冗余）；新写入只存 `build_context_observation_ref`，legacy 兼容读取。

## Scope

- `crates/pony-agent-core/src/agent/`：`session.rs`（append_turn 改造、cursor_version 退役）、`runtime/mod.rs` + `runtime/turn_persist.rs`（run_turn 事件化）、`turn_flow.rs`（StepStart/StepEnd、chunk step）、`turn_event.rs`（schema version envelope）、`sqlite_session.rs`（对拍、ref-only 写入）
- 前端：`src/lib/runtime/`（如 wire 有变更走显式版本化）
- OpenSpec change：`event-sourcing-closeout`（proposal/design/tasks + delta spec）

## Non-Goals

- 事件表历史数据迁移重写（legacy 数据靠 backfill 兼容）
- 投影 checkpoint 持久化 / 增量重放引擎（阶段 5 方向，另卡）
- suspended 缓冲 TTL、前端 in-flight 去重等 P2 打磨项（登记于 PA-094 任务卡后续清单）

## Acceptance Criteria

1. 同步 `run_turn` 与 streaming 入口产生等价事件流（同 turn 形态逐事件类型断言）
2. `append_turn` 路径不再整包 upsert 快照；写放大消除有量化证据（写次数/字节数前后对比）
3. 对拍测试：随机事件序列 → 事件折叠 vs blob 快照在约定字段集一致，豁免清单显式化且每项有理由
4. 多 hop turn（≥2 次 provider call + 工具调用）重建 timeline 与运行时产物结构一致（call_model 条目数 = hop 数）
5. 事件 payload 携带 schema_version；构造未知类型事件读取时 fail loud（必需）/跳过（ignorable）有测试
6. `cursor_version` 字段退役或标记 deprecated，四类 history-control command 不再依赖它做冲突检测（水位替代）
7. 新写入的 trace cache 行不含全量 observation payload（仅 ref）；既有测试全绿（core + src-tauri + 前端）

## Review Plan

- spec 3 路对抗审核 → 采纳修订 → 实现 → 实施后 3 路对抗审核（含 ox-alpha 模型通道）→ 调优 → 验证 → 收口归档

## Current Progress

- 2026-08-21：由 ADR 0008 完成度审核立卡（7 项缺口清单化）
- 2026-08-21：OpenSpec change 四件套创建（validate 通过）+ spec 两路对抗审核采纳修订——
  - ox-alpha（架构，PASS WITH REVISIONS）3 个正确性级缝隙全部纳入：①run_turn 早退路径发射责任枚举 + "每 turn:started 必有配对 turn:end" 不变式；②append_turn 取数时序（事件表提交先于缓冲清空的原子序 + (session_id, turn_id) 定位 + 兜底收窄为仅真无事件后端）；③水位单调性不变式（checkout/fork/squash 发射 history-control 事件使日志水位严格递增，消除 ABA 窗口）
  - general（可测性，PASS WITH REVISIONS）5 项修订全部纳入：schema_version 缺失 key 视为 v1 回填、Fallback 精确谓词、spec 自包含化（约定字段集+豁免清单入 delta 正文）、tasks 补 P1 测试缺口（failed/cancelled 等价、回退路径、legacy 分支、水位倒退、多 turn/legacy 混跑对拍）、写放大测量改双层指标（persist-command 结构性断言为主 + checkpoint 尺寸比例为辅）、豁免常量单源 + 反向探针防腐化
- **2026-08-21 实现（会话续接）**：
  - **#5 schema_version 契约**（完成）：`events.schema_version` seed/校验/回填四分支 + `load_turn_events_checked` fail loud + `IGNORABLE_EVENT_TYPES` 空常量（读取路径消费：清单内跳过、清单外 fail loud）+ SessionStore `event_stream_degraded` 标记与 `is_event_stream_degraded` 查询入口 + `event_schema_version_contract_four_branches` 测试（含 degraded 断言）。
  - **#1 run_turn 事件化**（完成）：turn:started/completed 全路径发射 + 7 处失败路径 `emit_sync_turn_failed`；`sync_turn_event_id` 一次性计算贯穿（nanos+进程内计数器）；终态信封取自发射本身（`emit_event` 返回 envelope），TurnResult/trace 信封与事件流同源序列号；等价性测试 `run_turn_and_start_turn_stream_produce_equivalent_event_sequences`（completed/failed 骨架一致 + 悬挂不变式）；端到端测试 `graph_sync_run_turn_persists_event_stream_end_to_end`（control_plane.run_turn → 生产通道 → SQLite 落盘 → 读回断言，重试吸收并行噪声）。
  - **实现期修复的 3 个正确性缺陷**（在途代码验证发现）：①`emit_stream_event` 调用参数数错位（编译不过）；②turn_id 动态 elapsed 计算导致 started/failed id 漂移（破坏配对不变式）+ Windows 计时器精度下同会话连续两轮 id 碰撞（trace 互相覆盖 + 事件 flush 失败——monitor-summary flake 根因）；③legacy `build_terminal_turn_event_envelope` 二次分配序列号（TurnResult sequence 与事件流错位；三处既有测试断言按新语义更新 seq=2）。
  - **测试基建**：测试多播 sink（`register_event_persist_test_sink`）——生产单槽通道可被并行 HostControlPlane 构建覆盖，多播 sink 独立存在；等价性测试按 session 隔离。**结构性候选改进**（登记）：生产单槽注册（overwrite 语义）在多 control-plane 并存场景下互相抢占，可考虑按 session 所有权路由的多通道模型。
  - **#4 StepStart/StepEnd + chunk step**（完成）：`TurnStreamEvent.step`（serde default，wire 兼容）+ `emit_stream_event_with_step`；followup 循环 hop 索引贯穿 delta 闭包；`build_step_start_event`（turn:started→StepStart{0}、turn:trace(calling_model) step≥1→followup StepStart）+ `build_step_end_events`（与 usage 相邻成对，仅 completed 结算）；delta chunk 按 payload.step 归属。测试：`multi_hop_turn_rebuilds_timeline_with_per_hop_call_model_entries`（streaming 3-hop 全断言 + sync usage 兜底变体）+ serde 往返。**顺带修复生产数据丢失 bug**：followup delta 闭包 session_id 原传 None——全部 followup chunk 以空 session 缓冲且永不 flush。
  - **#2 append_turn 投影化**（完成，含阶段 B 登记）：事件源物化（最近 turn 的 UserMessage/AssistantMessage 文本 + reasoning 随事件；attachments 暂经调用方参数——登记偏差）+ 取数时序（persist 闭包与 flush requester 均 commit 先于 clear；生产路径写锁前预提交缓冲——实现期发现持锁自锁死锁并修复，requester try_write 化）；兜底收窄（supports_turn_events 能力标志，三类可观测回退 + degraded fail loud panic）；增量持久化阶段 A（2×AppendMessage + UpdateSessionMeta + 单会话行落库，**save_store 整包写=0**）；characterization 快照防护（改造前录制、含 SQLite blob 往返）+ 物化防架空探针（caller≠事件文本）+ 纯函数选取测试。**登记待办**：阶段 B 会话行级按 facet 增量（transcript/memory/nodes 脱离行重写）、wal 尺寸基线采样。
  - **#3 对拍测试 + 豁免清单**（主体完成，场景矩阵 3/7 落地）：豁免清单常量化（projection.rs::parity 单一事实源，9 项附理由 + 取值器）+ 生产形态对拍 harness（control plane + SQLite 表读事件，重试抗抢占）+ 3 场景（无工具 sync/单工具流式/failed hook-fail）+ 反向探针 4 个 + 豁免迭代消费。**对拍驱动收敛修复 4 项**：投影 phase 设置（turn/end 按 reason）、ProviderUsage provider 字段取名称、call_tool label/state 同构、存储侧 timeline 补 return_result 条目。**登记待办**：多 hop/cancelled/squash/fork-checkout/multi-turn/legacy-mixed 场景接入对拍断言、build_context 元数据探针。
- **2026-08-22 会话续接：#3 cancelled 对拍验证完成，parity 6/6 全绿（修复链 7 项）**：
  - **生产缺陷修复**：`emit_stream_cancelled` 调用点 session_id 传 None——用户取消流式 turn 的 turn:cancelled 终态事件进空会话缓冲被 flush skip（事件流丢终态）；修复为携带 session_id。
  - **测试 harness 挂死修复**：cancelled-before-plan 无 HTTP 请求，`MockHttpServer::finish()` join 阻塞在 accept 的线程永挂；改 drop(server)。
  - **对拍收敛修复⑤**：投影 TurnEnd 结算时 Cancelled/Aborted turn 中未经 usage 结算的 call_model 条目标记 cancelled（中断调用不得显示 completed）。
  - **豁免登记**：terminal_no_usage_turn_provider_metadata（原 failed_turn_provider_metadata 扩展覆盖 cancelled）+ 双向反向探针。
  - **探针语义对齐 #4**：plan 前 cancelled 探针改断言"无虚构完成态"——step/start 真实存在于事件流（turn:started 恒发 StepStart{0}），重建条目来自事件而非 settle 补建。
  - **既有缺陷修复**：settle 兜底收窄后 build_context 前插 get_mut 静默丢失（无模型活动 turn 无既有 timeline 条目），改 entry API；`trace_projection_context_observation_sets_ref` 对齐收窄语义。
  - **稳定性**：parity 模块内串行锁 `PARITY_SCENARIO_LOCK`——生产 EVENT_PERSIST_REGISTRY 单槽被并行场景构建互抢是 multi_turn flake 根因。
- **2026-08-22 会话续接（二）：#6/#7/#3 收尾全部完成**：
  - **全量验证前置修复**：①fork 死锁——`emit_global_event` 在 history 命令持锁路径内联 flush 同线程重入 sessions 写锁挂死（history_restore_fork_switch 实测），改为仅入缓冲 + 四命令包装 drop 锁后补 flush；②并行饥饿——上述串行化锁反而使场景与全量套件竞争全局单槽永不翻盘，落实任务卡登记的结构性改进：**会话绑定路由**（`bind_event_persist_session`/`bind_event_flush_session`，cfg(test)，绑定优先回退默认槽，生产语义不变；串行化锁移除）；③入口确定性补提交：run_turn/start_turn_stream 返回前 flush（吸收 trace worker 毫秒级锁竞争）。**全量 core lib 863×4 连续全绿**。
  - **#6 cursor_version 退役**：四处 `bump_cursor_version` 删除改水位镜像；冲突检测 `reject_stale_cursor_version` 改比较权威源 `session.event_watermark`（错误信息 watermark 语义）；`finalize_event_watermark` 统一镜像 cursor_version/event_watermark。测试：四命令表驱动冲突测试 `pa095_history_command_conflict_detection_is_watermark_based` + checkout 水位严格递增/无 ABA 测试 `pa095_checkout_version_strictly_increases_with_watermark_no_aba`（SQLite 两批事件推进）。grep 盘点 61 处引用，既有测试全部 expected=None 无需迁移。前端 cursorVersion 不透明使用，wire 兼容。
  - **#7 trace cache ref-only**：`record_turn_trace_in_memory` 剥离 observation payload（R4b）；`backfill_trace_observation_ref_tx` flush 同事务双表 JSON 回填 ref（session_turn_traces.trace_data + normalized_turn_traces.raw_json）；requestFormat 断言盘点 16 处无需迁移。测试：`pa095_ref_only_trace_rows_backfill_and_legacy_reads` 四段覆盖（live 剥离 / 回填 / ref 加载一致 / legacy 内嵌可读）。
  - **#3 收尾**：新增 `parity_fork_checkout_branch_turns` 跨分支场景（fork/created 与 checkpoint/checkout 落盘探针 + fork seq > 首 turn/end 单调探针 + switch/fork 版本严格递增无 ABA 端到端断言 + 跨分支逐 turn 对拍）+ build_context provider 元数据反向探针（no_tool 场景）。登记偏差：squash 无生产入口（PA-091 前向事件类型，投影级已覆盖）；legacy-mixed 以存储级混跑覆盖（#7 测试同库双形态共存）。
  - **实施后对抗审核（三路独立子智能体：正确性回归 / 架构规范一致性 / 边界失败路径，结论均为"有条件通过"）——10 项代码修复全部落地**：
    - **ABBA 死锁残余**（边界路 P0/正确性路 P2）：persist 闭包持 buf mutex 调阻塞 flush × history 命令持 sessions.write 发射抢 buf mutex——重构为锁内聚合/终态批快照、锁外 flush 后重加锁移除。
    - **cursor 往返契约破坏**（架构路 F2/正确性路交叉确认）：命令响应版本取补 flush 后当前水位（`fresh_cursor_version` 只读访问器；load_history_cursor 有 ensure/save 副作用不可用于响应重建——实测会把 checkout 截断态修复回去）；fork-checkout 场景升级为 round-trip 断言（switch 携带 fork 响应版本必须通过）。
    - **#7 两处未达 spec**（架构路 P0）：timeline 条目内嵌 payload 三处源头剥离；读侧水合缺失补齐（`hydrate_trace_observation`）；ref 回填被异步 worker REPLACE 覆盖补内存附加（backend `load_observation_refs` + finalize 后 attach）。
    - **缓冲滞留两类**：匿名 turn 空 session_id 归一 DEFAULT_SESSION_ID；无事件后端 Unsupported 显式清除（原"保留可重试"为不可满足承诺）。
    - **其余**：graph 两入口补返回前 flush；绑定守卫 Arc::ptr_eq 解绑；生产单槽覆盖告警；存量 cursor_version≠watermark 一次性升级校正 SQL。
  - **登记偏差/残余风险**：(a) ABA 窗口在包装补 flush 失败时短暂重开；(b) append_turn degraded panic 在可中毒写锁链上（前序设计决策，后续卡评估 Result 化）；(c) display_message≠user_message 物化文本分叉；(d) 反向探针未覆盖全部豁免项（容差/跳过类以消费方式替代）；(e) 多客户端共享 SQLite 水位比较源为内存态（单写者假设）；(f) turn_id 复用场景 stop 请求残留（全局唯一前提下无害）。详见 tasks.md Validation Notes 审核记录。
- **验证（2026-08-22 终态）**：core lib **867×4 连续全绿**（审核修复后最终代码）；regression 三件套 + src-tauri lib 6 + vitest 399+10skip + cargo:check:shared 终版复跑通过；openspec validate --strict 通过。

## Next Action

- 归档本 change（canonical spec 同步）→ 提交准备。后续卡：#2 阶段 B 行级 facet 增量与 wal 基线、squash 生产入口、append_turn panic Result 化评估。

## Blockers

- 无（PA-091~094 已全部收口）

## Resume Hint

- 先读 `docs/decisions/0008-event-sourcing-evolution.md` 四阶段承诺原文，再对照本卡 Goal 逐项实施；缺口 #1/#2 是架构主线（事件权威闭环），#3 是验收证明，#4~#7 是契约收尾
