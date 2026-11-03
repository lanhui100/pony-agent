# Backlog

## 进行中

暂无

## 待办

- [ ] `B001` - P2: `TokenUsage` 序列化字段计数与实际字段对齐
  - 路径：`crates/pony-agent-core/src/agent/provider/mod.rs:348`
  - 描述：`serializer.serialize_struct("TokenUsage", 9)?` 声明容量 9 与实际序列化的 8 个字段不符（serde hint，无功能危害，但有微量序列化容量预分配漂移）。
  - 登记来源：wave-55 遗留债登记

- [ ] `B002` - P2: `weighted_turn_speed` 与 `record_turn_speed_truth` 补齐直接单测
  - 路径：`crates/pony-agent-core/src/agent/turn_flow.rs:2377,2421`
  - 描述：加权速度计算与真值落盘函数目前仅经 `TurnTraceGuard::drop` 派生路径被间接覆盖，缺乏针对多 hop / 零耗时 / 溢出等边界情况的直接单元测试断言。
  - 登记来源：wave-55 遗留债登记

- [ ] `B003` - P2: 补齐「跨轮 calls 继承」回归测试用例
  - 路径：`crates/pony-agent-core/src/agent/turn_flow.rs` 及相关 stream/sync 运行时
  - 描述：跨轮对话与历史继续流转时的 `calls` 继承与累计逻辑缺乏专门的隔离回归用例验证，需补齐端到端契约与状态重载测试。
  - 登记来源：wave-55 遗留债登记

- [ ] `B005` - P1: 真·promote：Run 超时把运行中命令转后台 job（DSH 对齐）
  - 路径：`crates/pony-agent-core/src/agent/jobs.rs`（新增 promote 适配层）、`crates/pony-agent-core/src/agent/tools.rs:3011,3122-3176`
  - 描述：侦察结论（.dev-team，2026-11-04）——Run 的进程所有权在 `ProcessManager`（`process.rs:306`）、`poll` 为「取走即清空」（`:869-872`）、`kill_after` 无 disarm API（`:433-443`）、取消 watcher 随函数返回 drop。需在 jobs.rs 内做 `JobKind::Promoted` 适配层 + 生命周期重构；在此之前超时仍为 kill 语义。
  - 登记来源：tool-timeout-policy 波次一（契约 v2 明确不做）

- [ ] `B006` - P2: Run 超时三处 ADVISORY（L3 审查 2026-11-04 遗留）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs:3006-3033`、`crates/pony-agent-core/src/agent/process.rs:433-443`、`crates/pony-agent-core/src/agent/orchestration.rs:334`
  - 描述：① 轮询 deadline 与 `kill_after` 存在窄竞态：deadline 前最后一次 poll 若跨过杀点，会读成 Exited → `non_zero_exit`（丢 timeout hint，且指纹带 timeoutMs），高负载下可让 `tool_timeout_policy_test` 偶发红；② `DEFAULT_RUN_TIMEOUT_MS=120_000` 与 `subagent_turn_timeout_ms=120_000` 预算碰撞，子代理内默认 Run 挂死会先撞子代理超时，拿不到工具级证据；③ `kill_after` 无 disarm，每次 Run 留一条睡满 timeout 的线程（默认 120s / 显式 600s）。
  - 登记来源：`.dev-team/review-tool-timeout.json`（verdict PASS, 3 ADVISORY）

## 已完成

- [x] `B004` - P1: Run 超时提示指向后台路径（CT-5 契约演进波次）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs`（`classify_timeout_hint`，约 :7963-7978）
  - 描述：超时 hint 原只给「缩短命令 / 更大 timeoutMs」，不提已 ModelVisible 的 `job_start`/`job_output`；trace `pa_01a1a134` turn-0 正是超时后闷头重试。
  - 验收：契约 v3（CT-5/CT-5a）；冻结断言迁移 `71b9280`（wave9_green 逐字文案 + 3 标记断言）；实现 `TD_B004`（单一真源文案、`pub classify_timeout_hint`、schema `:5864` 与 `browser-preview.ts:35` 逐字同步）；`tool_timeout_policy_test` 15 passed、`wave9_green_acceptance_test` 2 passed。

- [ ] `B005` - P1: 真·promote：Run 超时把运行中命令转后台 job（DSH 对齐）
  - 路径：`crates/pony-agent-core/src/agent/jobs.rs`（新增 promote 适配层）、`crates/pony-agent-core/src/agent/tools.rs:3011,3122-3176`
  - 描述：侦察结论（.dev-team，2026-11-04）——Run 的进程所有权在 `ProcessManager`（`process.rs:306`）、`poll` 为「取走即清空」（`:869-872`）、`kill_after` 无 disarm API（`:433-443`）、取消 watcher 随函数返回 drop。需在 jobs.rs 内做 `JobKind::Promoted` 适配层 + 生命周期重构；在此之前超时仍为 kill 语义。
  - 登记来源：tool-timeout-policy 波次一（契约 v2 明确不做）

- [ ] `B006` - P2: Run 超时三处 ADVISORY（L3 审查 2026-11-04 遗留）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs:3006-3033`、`crates/pony-agent-core/src/agent/process.rs:433-443`、`crates/pony-agent-core/src/agent/orchestration.rs:334`
  - 描述：① 轮询 deadline 与 `kill_after` 存在窄竞态：deadline 前最后一次 poll 若跨过杀点，会读成 Exited → `non_zero_exit`（丢 timeout hint，且指纹带 timeoutMs），高负载下可让 `tool_timeout_policy_test` 偶发红；② `DEFAULT_RUN_TIMEOUT_MS=120_000` 与 `subagent_turn_timeout_ms=120_000` 预算碰撞，子代理内默认 Run 挂死会先撞子代理超时，拿不到工具级证据；③ `kill_after` 无 disarm，每次 Run 留一条睡满 timeout 的线程（默认 120s / 显式 600s）。
  - 登记来源：`.dev-team/review-tool-timeout.json`（verdict PASS, 3 ADVISORY）

## 已完成

- [x] `B004` - P1: Run 超时提示指向后台路径（CT-5 契约演进波次）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs`（`classify_timeout_hint`，约 :7961）；
  - 描述：超时 hint 目前只给「缩短命令 / 更大 timeoutMs」，不提已 ModelVisible 的 `job_start`/`job_output`；trace `pa_01a1a134` turn-0 正是超时后闷头重试。改文案会撞冻结用例 `crates/pony-agent-core/tests/wave9_green_acceptance_test.rs:143`（逐字断言默认文案），已完成：契约 v3（CT-5/CT-5a）+ 冻结断言迁移 commit `71b9280` + 实现 commit `TD_B004`。
  - 登记来源：tool-timeout-policy 波次一（契约 v2 明确延后）

- [x] `B000` - P0: 修复 `npx vitest run` 全量门禁偶发 exit=1（ReferenceError: window is not defined @ src/stores/runtime.ts:904）
  - 路径：`src/lib/runtime/utils.ts` 与 `src/stores/runtime.ts`
  - 描述：通过环境销毁检测中断延迟宏任务回调，收口 scheduleDeferredPersist/cancelDeferredPersist 对 window 的裸解引用。
  - 验收：10 次全量连跑 exit=0 且 Errors 0。
