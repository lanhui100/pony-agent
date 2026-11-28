# Agent Note: 退役 `plan/update` 线缆事件与 `UiEvent::PlanUpdated`（additive-retire）

Status: implemented

## Problem

ADR `2026-11-27-plan-mode-dsh-alignment-and-task-count` 把 Plan 收敛为**会话级协作模式**后，
Plan 状态一律经控制面 `plan_mode_get` 拉取（`crates/pony-tui/src/tui/projection.rs` 的
`sync_plan_from_projection`，抽屉打开与 500ms 常驻刷新共用）。旧的双通路因此全部悬空：

- `TurnEvent::PlanUpdate`（wire 名 `plan/update`，`crates/pony-protocol/src/turn_event.rs`）
  **没有任何 emit 点**，只剩变体定义、wire 名映射、`turn_id()` 归类与一条折叠单测；
- `pony-storage` 的 `PlanProjectionState` 只有自身折叠单测，无生产消费者（会话视图从不读它）；
- TUI 的 `UiEvent::PlanUpdated(PlanModeState)` 同样只剩变体定义 + 一个消费分支
  （`crates/pony-tui/src/tui.rs` 的事件循环），无生产者。

三处都是「看起来还在工作的死路径」：读者无法从代码判断 Plan 状态到底走哪条通路，改动 Plan
时容易顺手改错分支。B1795824704407 登记为死事件清理项。

## Decision

按 **additive-retire** 处理，不做直接删除：

1. **`TurnEvent::PlanUpdate` 保留变体与 wire 名**。`TurnEvent` 是会话日志（SQLite + JSONL）的
   线缆枚举，删除变体会让含 `plan/update` 的**历史日志反序列化失败**——那是不可恢复的数据损失。
   变体上加 retired 标注：新代码**不得再发**，投影仍能读历史日志。
2. **`PlanProjectionState` 与折叠单测保留**。它是历史日志的读取路径，不是死代码；
   一旦删除，含旧事件的会话在 refold 时会撞到未知变体。
3. **删除 `UiEvent::PlanUpdated` 变体与其消费分支**。`UiEvent` 是**进程内**枚举，不落盘、
   不跨版本，没有兼容包袱；留一个无生产者的变体只会让 `match` 分支看起来像活的。
4. 复活条件与检索收据见下节。

## Removed surface

- `crates/pony-tui/src/tui.rs`：`UiEvent::PlanUpdated(PlanModeState)` 变体 + 事件循环里的
  `UiEvent::PlanUpdated(plan) => { state.update_plan(plan); }` 分支。

检索收据（删除前确认零生产者）：

```
$ rg -n 'UiEvent::PlanUpdated|PlanUpdated\(' crates --type rust
crates/pony-tui/src/tui.rs:328:    PlanUpdated(PlanModeState),
crates/pony-tui/src/tui.rs:13846:                UiEvent::PlanUpdated(plan) => {
```

两处都只有「定义」与「消费」，**没有任何构造点**。

## Reintroduction conditions

同时满足以下三条才把进程内事件通路接回来（而不是「以后可能需要」）：

1. Plan 状态出现**无法由控制面拉取**的推式来源（例如外部 agent 主动推 Plan 变更，且
   `plan_mode_get` 的轮询周期成为可观测的用户体验缺陷）；
2. 该推式来源**不落在持久化线缆枚举**上（`TurnEvent` 仍只做历史读取），否则需要先解决
   日志版本兼容，而不是复活一个进程内变体；
3. 复活时同时补一条「有生产者」的守卫用例（构造该事件并断言状态被更新），
   否则新变体会再次退化成死路径。

## Alternatives considered

- **连 `TurnEvent::PlanUpdate` 一起删掉**：会让含 `plan/update` 的历史会话日志反序列化失败，
  等于用「代码更干净」换「老会话打不开」。线缆枚举只能 additive-retire。
- **给 `PlanProjectionState` 加 `#[deprecated]` 并删除折叠单测**：单测是历史读取路径的唯一证据，
  删掉后没人知道这条投影还该不该工作；`deprecated` 也无法阻止 refold 路径被误删。
- **保留 `UiEvent::PlanUpdated` 并标注「保留供未来使用」**：正是本仓明令禁止的怀旧式保留——
  无生产者的 match 分支会在下次改动时被误认为活路径。
- **把 Plan 状态继续走 `TurnEvent` 双写（控制面 + 事件）**：两个真源必然漂移，
  且与 ADR 2026-11-27 的「Plan 是会话级模式、不是事件流状态」直接冲突。

## Consequences

- Plan 状态的唯一真源是控制面投影；`plan/update` 仅作为历史日志的读取兼容面存在。
- 旧会话仍可正常 refold（`PlanProjectionState` 在册）。
- 新增守卫：`cargo check -p pony-tui --all-targets` 会因无生产者的构造点而暴露复活风险
  （变体已不存在，任何 `UiEvent::PlanUpdated` 写法直接编译失败）。
