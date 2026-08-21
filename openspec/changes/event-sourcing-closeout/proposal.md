# event-sourcing-closeout

## Background

ADR 0008（`docs/decisions/0008-event-sourcing-evolution.md`）四阶段主体已落地（PA-091~094），但 2026-08-21 对照 ADR 原文的完成度审核确认 **7 项承诺缺口**，正式立卡 PA-095。本 change 承接这些缺口的实施：

1. **同步 `run_turn` 绕过事件流**（P0）：`src-tauri/lib.rs:56`（Tauri command）、`non_tauri_harness.rs:105`、graph sync fallback（control_plane:1857）三个生产入口直接写 history/trace，零事件产生——这些 turn 无法从事件重建，事件权威在生产入口上不成立。
2. **`append_turn` 未改造为事件折叠**（P0，ADR 阶段 2 核心承诺）：仍直接 push history + 触发整包快照持久化；ADR 明示"append_turn 从 'push 消息+整包快照' 改为 'append 事件+增量折叠投影'"。
3. **blob↔事件折叠对拍测试缺失**（P1，ADR 阶段 2 验收承诺）：验收要求"从事件重建的快照与 blob 快照逐字段一致"，现状仅有序列化往返测试；已知结构性分歧（prepare_retrieval 条目、provider 元数据、title/session_id、failed 末 hop state 等）未固化为显式豁免清单。
4. **StepStart/StepEnd 不发射 + AssistantChunk 固定 step=0**（P1）：多 hop turn 的模型输出全部聚合到首个 call_model，timeline 重建与运行时产物结构性分歧（运行时产 N+1 个 call_model 条目，折叠只产 1 个）。
5. **`event_schema_version` 仅定义常量**（P1，ADR 阶段 1 承诺）：`EVENT_SCHEMA_VERSION=1` 从未写入 store_metadata、读取无校验、未知事件静默跳过——违背"未知必需事件 fail loud"约定。
6. **`cursor_version` 未退役**（P2，ADR 阶段 3 承诺）：ADR 明示"cursor_version 乐观锁退役（seq 水位本身就是版本）"，现状 session.rs/control_plane/sqlite_session/前端共 50+ 处引用，双版本机制并存。
7. **trace cache 仍内嵌全量 observation**（P2，spec R4b 违例）：live 路径 `record_turn_trace` 把完整 build_context_observation 写入 raw_json/blob（三处冗余）；事件外置只覆盖新事件路径。

另发现收口事务缺口：PA-091/092/093 的 OpenSpec changes（`turn-event-log`、`session-projection-layer`、`checkpoint-event-referencing`）仍在活跃区未归档。

## Goals

- 生产入口全覆盖：同步 `run_turn` 产生与 streaming 入口等价的事件流。
- history 由事件派生：`append_turn` 降级为投影物化步骤，消除整包 upsert 写放大。
- 重建等价可证明：对拍测试 + 显式豁免清单固化"重建==存储"的口径。
- 多 hop timeline 可重建：StepStart/StepEnd 事件闭环 + chunk 携带逻辑 step。
- 版本契约落地：schema_version 写入/校验/fail loud。
- 单一版本机制：cursor_version 退役，水位即版本。
- 大字段单份存储：trace cache 新数据 ref-only。

## Non-goals

- 事件表历史数据迁移重写（legacy 数据靠 backfill 兼容）。
- 投影 checkpoint 持久化 / 增量重放引擎（阶段 5 方向，另立卡）。
- suspended 缓冲 TTL、前端 in-flight 去重、chunk 聚合 O(n²) 优化等打磨项（登记于 PA-094 任务卡后续清单）。
- 事件查询 API、跨会话聚合查询。

## Scope

- `crates/pony-agent-core/src/agent/`：
  - `runtime/mod.rs` + `runtime/turn_persist.rs`：run_turn 事件化（NoopSink + emit_event 复用）。
  - `session.rs`：append_turn 投影化改造、cursor_version 退役。
  - `turn_flow.rs`：StepStart/StepEnd 映射、chunk step 透传、failed/cancelled 事件补齐。
  - `turn_event.rs`：schema version 契约。
  - `sqlite_session.rs`：schema_version 存取、对拍支持、trace cache ref 回写。
- 前端：`src/types/runtime.ts`、`src/lib/runtime/sessions.ts`（cursorVersion 语义切换，wire 字段名不变）。
- OpenSpec 收口：归档 `turn-event-log` / `session-projection-layer` / `checkpoint-event-referencing`。

## Risks

- **append_turn 改造影响面大**（89 处调用方）：采用保守策略——签名不变，内部改为"从事件折叠物化 history"，调用方无感；分两个子阶段（先事件源物化、后去整包写）。
- **run_turn 无 sink**：`TurnEventSink` 已有 Noop 默认实现且事件持久化走全局注册通道（与 sink 无关），run_turn 内部用 NoopSink 发射即可，前端推送不受影响（同步 API 本就无流式推送）。
- **cursor_version 退役的兼容**：wire 字段名保留，值来源切换为 event_watermark（水位即版本），前端零改动；CAS 逻辑改为水位比较。
- **对拍的双标准**：沿用 PA-094 确立的"事件权威"原则——存储记录是缓存，豁免清单显式化（时钟/序列/装饰性条目）。

## Validation

- 单元：run_turn 事件序列断言（逐事件类型）、schema_version 校验（fail loud/ignorable）、StepStart/StepEnd 映射、多 hop 折叠（call_model 条目数 = hop 数）。
- 对拍：随机/矩阵事件序列 → 事件折叠 vs blob 快照在约定字段集一致（豁免清单外零差异）。
- 集成：Tauri 同步 command 端到端产生事件；清 trace 表重启后同步入口的 turn 可完整重建。
- 回归：core + src-tauri + 前端全量测试全绿；验收命令 `npm run cargo:test` + `npm run verify`。
