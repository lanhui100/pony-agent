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

- [ ] `B008` - P0: promoted 作业的 `ManagedProcess` 条目永不回收（每 run 泄漏 1 条目 + Windows Job 句柄）
  - 路径：`crates/pony-agent-core/src/agent/process.rs`（poll 不 remove）、`tools.rs` promote 早返回跳过二次 kill、`ProcessManager::shutdown` 零生产调用方
  - 描述：L3-A BLOCKING。promote 后 `legacy_run_session_id()` 每 Run 唯一，`shutdown(session)` 无生产调用方 ⇒ 设计所称「由 shutdown 收口」不可达；promoted 进程只能靠模型 `job_kill`。
  - 登记来源：`.dev-team/review-promote-A.json`（2026-11-04）

- [ ] `B009` - P0: promote 后取消失效（用户 Stop 停不下来）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs:3004`（`CommandCancelWatcher` 随函数返回 drop）
  - 描述：L3-A BLOCKING，实测 cancel 后 `sleep 5` 仍跑完 exit 0。需把 turn 取消语义延续到 promoted 作业（或把 promoted 注册进可取消集合）。

- [ ] `B010` - P0: promote 成功摘掉 `error.code=timeout`，三道防线失去输入
  - 路径：`crates/pony-agent-core/src/agent/runtime/stream_support.rs:580/:591`、`runtime/recovery.rs:180`
  - 描述：L3-B BLOCKING。连续失败熔断、超时指纹归一、非幂等盲重试防护均以 `status=="error"` 为前提；promote 改为 ok 后三者在「命令超时」场景永不触发。这是 promote 默认关闭的主因（契约 CT-P7）。

- [ ] `B011` - P1: `timedOut/promoted` 无消费方，UI/trace 把仍在跑的命令显示为「成功完成」
  - 路径：`src/components/chat/WorkspaceTurnItem.vue:631`、`src/lib/runtime/trajectory-core.ts:320`、`src/lib/runtime/trace.ts:378`；`ok:true` + `exitCode:null` 破坏成功路径不变量
  - 描述：L3-B。需在 UI/trace/遥测补 promoted 呈现（可停入口、后台标记），并明确 exitCode 语义。

- [ ] `B012` - P0: job 层无会话归属（跨会话 list/read/kill）；Windows `.cmd` 残留含命令原文
  - 路径：`crates/pony-agent-core/src/agent/jobs.rs`（全局 REGISTRY、`JobListArgs{}` 空）、`tools.rs` Windows 批处理写入与 promote 早返回跳过删除
  - 描述：L3-B BLOCKING。`ProcessManager` 有归属检查但 job 层没有；同进程其它会话/subagent 可枚举、读取、终止他人作业与输出。`.cmd` 内含整条命令原文且无清扫器。
  - 登记来源：`.dev-team/review-promote-B.json`（2026-11-04）

## 已完成

- [x] `B005` - P1: 真·promote：Run 超时把运行中命令转后台 job（DSH 对齐）
  - 路径：`crates/pony-agent-core/src/agent/jobs.rs`（新增 promote 适配层）、`crates/pony-agent-core/src/agent/tools.rs:3011,3122-3176`
  - 描述：侦察结论（.dev-team，2026-11-04）——Run 的进程所有权在 `ProcessManager`（`process.rs:306`）、`poll` 为「取走即清空」（`:869-872`）、`kill_after` 无 disarm API（`:433-443`）、取消 watcher 随函数返回 drop。需在 jobs.rs 内做 `JobKind::Promoted` 适配层 + 生命周期重构；在此之前超时仍为 kill 语义。
  - 登记来源：tool-timeout-policy 波次一（契约 v2 明确不做）

- [ ] `B006` - P2: Run 超时三处 ADVISORY（L3 审查 2026-11-04 遗留）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs:3006-3033`、`crates/pony-agent-core/src/agent/process.rs:433-443`、`crates/pony-agent-core/src/agent/orchestration.rs:334`
  - 描述：① 轮询 deadline 与 `kill_after` 存在窄竞态：deadline 前最后一次 poll 若跨过杀点，会读成 Exited → `non_zero_exit`（丢 timeout hint，且指纹带 timeoutMs），高负载下可让 `tool_timeout_policy_test` 偶发红；② `DEFAULT_RUN_TIMEOUT_MS=120_000` 与 `subagent_turn_timeout_ms=120_000` 预算碰撞，子代理内默认 Run 挂死会先撞子代理超时，拿不到工具级证据；③ `kill_after` 无 disarm，每次 Run 留一条睡满 timeout 的线程（默认 120s / 显式 600s）。
  - 登记来源：`.dev-team/review-tool-timeout.json`（verdict PASS, 3 ADVISORY）
  - 结果：已实现（process.rs `disarm_kill_after`、jobs.rs `JobKind::Promoted`/`register_promoted`、tools.rs 超时分支分流）；L3-A/B 双路复核判 FAIL（泄漏/截断静默丢失/取消失效/熔断失去输入/跨会话越权/`.cmd` 残留），故改 **默认关闭**（`PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT=1` 开启），缺陷拆分为 B008–B012。修复后应重新评估默认开启。

- [x] `B004` - P1: Run 超时提示指向后台路径（CT-5 契约演进波次）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs`（`classify_timeout_hint`，约 :7963-7978）
  - 描述：超时 hint 原只给「缩短命令 / 更大 timeoutMs」，不提已 ModelVisible 的 `job_start`/`job_output`；trace `pa_01a1a134` turn-0 正是超时后闷头重试。
  - 验收：契约 v3（CT-5/CT-5a）；冻结断言迁移 `71b9280`（wave9_green 逐字文案 + 3 标记断言）；实现 `7675017`（单一真源文案、`pub classify_timeout_hint`、schema `:5864` 与 `browser-preview.ts:35` 逐字同步）；`tool_timeout_policy_test` 15 passed、`wave9_green_acceptance_test` 2 passed。

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
  - 描述：超时 hint 目前只给「缩短命令 / 更大 timeoutMs」，不提已 ModelVisible 的 `job_start`/`job_output`；trace `pa_01a1a134` turn-0 正是超时后闷头重试。改文案会撞冻结用例 `crates/pony-agent-core/tests/wave9_green_acceptance_test.rs:143`（逐字断言默认文案），已完成：契约 v3（CT-5/CT-5a）+ 冻结断言迁移 commit `71b9280` + 实现 commit `7675017`。
  - 登记来源：tool-timeout-policy 波次一（契约 v2 明确延后）

- [x] `B000` - P0: 修复 `npx vitest run` 全量门禁偶发 exit=1（ReferenceError: window is not defined @ src/stores/runtime.ts:904）
  - 路径：`src/lib/runtime/utils.ts` 与 `src/stores/runtime.ts`
  - 描述：通过环境销毁检测中断延迟宏任务回调，收口 scheduleDeferredPersist/cancelDeferredPersist 对 window 的裸解引用。
  - 验收：10 次全量连跑 exit=0 且 Errors 0。
