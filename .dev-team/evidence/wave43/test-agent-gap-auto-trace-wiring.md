# Test Agent 观察项：自动 turn 终态 → OTel span 的**运行时接线**缺失

日期：2026-10-24 17:32 · 提交人：Test Agent（不属写域，仅登记事实，等 Lead 裁决）

## 事实（机械证据）
1. `crates/pony-agent-core/src/agent/turn_flow.rs` 的 `emit_event` 终态分支只留下注释：
   ```
   // D6 迁移：PonySentry 自动 trace 上报已下线；agent trace 由 OTel span 承担
   // （门禁关闭时零 span 分配、零 egress，见 `agent::otel`）。
   ```
   **没有任何 `otel::` 调用**（`grep -n "otel::" turn_flow.rs` → 0 命中）。
2. 全仓生产代码中 `TurnScope` 的构造点：**0 处**（仅 `tests/otel_trace_migration_test.rs`、
   `tests/otel_trace_adversarial.rs` 使用）。
3. `llm_span_attributes` / `tool_span_attributes` 在生产代码中同样 0 处调用（仅单测）。
4. 生产代码唯一的 OTel 出口是手动上传：
   `control_plane/query_commands.rs:61 crate::agent::otel::force_export_session_trace(...)`。

## 影响
- 现状 = **只有「用户点上传」会产生 span**；正常 Agent 回合（即使全局开关已开启）
  不会产生 `agent.turn` / `llm.chat` / `tool.call` span ⇒ plan §1 目标 1（Turn/Tool/LLM 映射为
  OTel span 树）与 D2「开关打开即上报」在运行时未闭合。
- 契约矩阵 OT-2 只验证 `TurnScope` seam（测试自己构造 scope），**无法证明**主链路真的调用它；
  这是契约层覆盖盲区（§6 未登记），不是测试实现缺陷。

## 建议（供 Lead 裁决，二选一）
- **(a) 判定为 task-3 未完成**：要求 executor-core 在 `emit_event` 终态 / LLM 调用 / 工具调用处
  接入 `TurnScope` 与 `llm_span_attributes`/`tool_span_attributes`，并补一条「主链路接线」静态断言
  （Test Agent 侧可加：`turn_flow.rs` 必须出现 `otel::TurnScope::new(` 且位于终态分支）。
- **(b) 明确降级为 Non-Goal**：若本波只交付「接口 + 手动上传 + 开关」，必须在 plan §1 与契约矩阵
  §6 显式改写目标并登记盲区，避免「实现完成」的误判。

> 当前 Test Agent 不擅自补该断言：它会把本已达成的 21/21 绿相重新变红，属于**新增验收面**，
> 须经 Lead 裁决后再落。
