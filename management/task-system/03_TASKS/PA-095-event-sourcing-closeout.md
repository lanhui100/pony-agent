# PA-095 事件溯源收尾（ADR 0008 承诺缺口关闭）

## Basic Info

- ID: PA-095
- Status: Ready
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-21
- Updated At: 2026-08-21
- OpenSpec Change: `openspec/changes/event-sourcing-closeout/`（validate 通过）
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

## Next Action

- 按实施顺序启动实现：#5 缺失-key 语义定案 → #1 run_turn 事件化 → #4 StepStart/StepEnd → #2 append_turn 投影化（characterization 快照防护）→ #3 对拍 → #6/#7 收尾 → #8 归档

## Blockers

- 无（PA-091~094 已全部收口）

## Resume Hint

- 先读 `docs/decisions/0008-event-sourcing-evolution.md` 四阶段承诺原文，再对照本卡 Goal 逐项实施；缺口 #1/#2 是架构主线（事件权威闭环），#3 是验收证明，#4~#7 是契约收尾
