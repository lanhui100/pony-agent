# Agent Note: 降级收尾独立成第四种历史行种类（Degraded）

Status: implemented

## Problem

`HistoryMessageKind` 只有 `Cancelled / Interrupted / Error` 三值，而降级收尾轮（provider 本地兜底
摘要，`TurnEndReason::Degraded`）在两处写入点被硬写成 `Interrupted`：

- `crates/pony-agent-core/src/agent/projection.rs` 的 `upgrade_degraded_assistant_message`
  （`message.kind = Some(HistoryMessageKind::Interrupted)`）与 `checkpoint_kind_for` 的 `_` 兜底臂；
- `crates/pony-agent-core/src/agent/runtime/turn_persist.rs` 的
  `persist_completed_turn_outcome`（`(text, transcript, Some(HistoryMessageKind::Interrupted))`）。

展示层（`crates/pony-agent-core/src/bin/tui/errors.rs` 的 `interrupted_record_status_line`）对
`Interrupted` 一律渲染「上一轮已中断，已保留已执行的步骤记录。」——于是**降级成功被读成「没跑完」**。

这不是措辞问题，是续跑决策失真：`Interrupted` 的语义是「本轮没跑完就被进程终止」，用户看到它
倾向原样重跑；而降级轮**已经交付了内容**（工具已执行 + 本地摘要），正确动作是「读一下、看是否够用」。
两者共用一个 kind，展示层再准也只能说错话。

B217（ADR `2026-11-17-turn-end-reason-interrupted.md`）已把 `Error` 从 `Interrupted` 里分出，
本条的 `Degraded` 是同一治理动作的第三步：**终态有几种续跑决策，历史行就有几种种类**。

## Decision

1. **库层立法**：`HistoryMessageKind` 新增 `Degraded`（wire 值 `"degraded"`，serde snake_case）。
   变体 doc 显式写明「跑完了、降级交付」，与 `Interrupted`（没跑完）划界。
2. **fail-soft 读**：手写 `Deserialize`（与 `TurnEndReason` 同口径）——`"cancelled" / "error" /
   "degraded"` 各自成支，**`"interrupted"` 与未知 wire 值同落 `Interrupted`**（该臂不单列，
   否则 `clippy::match_same_arms` 判红）。旧二进制读新日志不会因新值炸开整条历史。
3. **投影映射四态分明**：`checkpoint_kind_for` 显式列 `Degraded => HistoryMessageKind::Degraded`，
   不再落进 `_` 兜底；`upgrade_degraded_assistant_message` 就地升级时同样标 `Degraded`。
4. **写侧同改**：`persist_completed_turn_outcome` 的降级分支落 `Degraded`，与投影侧同源。
5. **展示层四句分流**：`errors.rs` 新增 `DEGRADED_TURN_LINE`（"上一轮以降级方式收尾，已保留已执行
   的步骤记录。"），`interrupted_record_status_line` 增 `Degraded` 分支。措辞刻意不含「中断」「失败」——
   降级轮有交付物。
6. **跨端同步**：`src/types/runtime.ts` 的 `kind` 联合类型补 `"error" | "degraded"`；
   `src/lib/runtime/cancelled-turn.ts` 补 `INTERRUPTED_ERROR_LINE` / `DEGRADED_TURN_LINE` 与两条分支，
   文案与 Rust 逐字一致（此前桌面端缺 `"error"`，同一份历史在两端说法不同）。

## Alternatives considered

- **只改文案、不动枚举（把 `Interrupted` 的降级轮单独嗅探文本）**：否决。降级轮的正文形态与
  失败/中断检查点同形（都以 `[中断轮次检查点]` 开头），文本嗅探无法区分三者；且这等于把已经
  结构化的终态语义又推回字符串判断，与 B217 的立法方向相反。
- **复用 `TurnEndReason::Degraded` 的布尔标记（在 `TurnHistoryMessage` 加 `degraded: Option<bool>`）**：
  否决。已有 `kind` 是历史行种类的唯一语义位，再开平行字段就是两套真源，读侧要做二次分派，
  且旧数据还得回退嗅探——扩一个枚举变体更小更硬。
- **把 `Degraded` 归入 `Completed`、不合成检查点**：否决。降级轮必须留痕（否则重启后只剩一条悬空
  user，B219 已立法），且它确实不是正常完成；归入 `Completed` 会让「本轮完成 · 用时 …」这类
  摘要行谎称质量正常。
- **只改 TUI 不改前端**：否决。桌面端 `interruptedRecordStatusLine` 是同一判定函数的第二实现，
  只改一端就是「同一份历史两种说法」，B275 记录了这个缺陷。

## Consequences

- 历史行种类从 3 值扩到 4 值，`kind` 是**持久化契约的加法**变更：新读端能读旧数据（旧数据只有
  前三个值），旧读端读新数据时 `"degraded"` 落 `Interrupted` 兜底（仍能显示一行状态，只是文案
  退化成「已中断」）——降级方向安全。
- `crates/pony-agent-core/tests/b219_degraded_turn_acceptance.rs` 的 Assert 2 随契约迁移：
  「降级轮 kind 与中断轮同形」改为「降级轮 kind 独立为 `Degraded`」。这是本条的**主动契约变更**，
  不是回归。
- 回归口径：`cargo test -p pony-agent-core --lib history_projection_keeps_degraded_apart_from_interrupted`、
  `cargo test -p pony-agent-core --bin pony degraded_kind_has_its_own_line`、
  `cargo test -p pony-agent-core --bin pony degraded_kind_wire_round_trips_and_unknown_is_fail_soft`、
  `cargo test -p pony-agent-core --test b219_degraded_turn_acceptance`、
  `npx vitest run tests/interrupted-record-status.spec.ts`。
- 未覆盖：`normalized_messages` 表没有 `kind` 列（走该路径读回的历史恒为 `None`），降级轮的
  结构化身份依赖节点快照/事件重建；这是既有架构约束，本条不扩表。

## Related

- `2026-11-17-turn-end-reason-interrupted.md`（B217：`Error` 从 `Interrupted` 分出，本条同族第三步）
- `2026-11-16-degraded-turn-reason-and-trace.md`（`Degraded` 终态的引入与 trace `providerFallback`）
- `2026-11-18-degraded-turn-abort-diagnosis-and-fix.md`（降级轮中止诊断）
- `2026-11-22-turn-end-status-noise.md`（B272/B273：resume 状态行只判末轮 + 停止原因不再占消息区）
