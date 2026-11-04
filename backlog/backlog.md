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


- [x] `B011` - P1: `timedOut/promoted` 无消费方，UI/trace 把仍在跑的命令显示为「成功完成」
  - 路径：`src/components/chat/WorkspaceTurnItem.vue:631`、`src/lib/runtime/trajectory-core.ts:320`、`src/lib/runtime/trace.ts:378`；`ok:true` + `exitCode:null` 破坏成功路径不变量
  - 描述：L3-B。需在 UI/trace/遥测补 promoted 呈现（可停入口、后台标记），并明确 exitCode 语义。
  - 结果：已修复（最小可见切片）。新增 `src/lib/runtime/tool-result-meta.ts`（`parsePromotedToolResult` fail-safe + `promotedToolBadgeLabel`）；`WorkspaceTurnItem.vue` 在 done/error 分支前渲染「已转后台」徽标（promoted 时不再显示成功/失败图标），结果文本取自 `turn.tools` 中 role=tool 的 `content`（即后端 ToolResult.output）。验收 `tests/tool-result-meta.spec.ts` 8 passed、`runtime-store` 119、`typecheck` Exit 0、全量 `test:unit` 139 files/1316 passed。
  - 未做（状态机语义）：`trajectory-core.ts`/`trace.ts` 仍把 promoted 记为 completed，如需「后台运行中」状态需独立卡（会牵动 V10 冻结快照）。

- [x] `B010` - P0: promote 成功摘掉 `error.code=timeout`，三道防线失去输入
  - 路径：`crates/pony-agent-core/src/agent/runtime/stream_support.rs:580/:591`、`runtime/recovery.rs:180`
  - 描述：L3-B BLOCKING。连续失败熔断、超时指纹归一、非幂等盲重试防护均以 `status=="error"` 为前提；promote 改为 ok 后三者在「命令超时」场景永不触发。这是 promote 默认关闭的主因（契约 CT-P7）。
  - 结果：已修复。`tool_failure_signal` 对 `status==ok && timedOut==true`（promote）产出与 error 路径逐字相同的 `(tool, 归一化指纹, "timeout")` ⇒ 指纹归一闪与两道连续失败闸重获输入；promote 文案补「不要重复执行同一命令」。未改阈值/错误码/签名。验收 `timeout_signal_promote_test` 3 passed（红相：promote ⇒ None）。

- [x] `B012` - P0: job 层无会话归属（跨会话 list/read/kill）；Windows `.cmd` 残留含命令原文
  - 路径：`crates/pony-agent-core/src/agent/jobs.rs`（全局 REGISTRY、`JobListArgs{}` 空）、`tools.rs` Windows 批处理写入与 promote 早返回跳过删除
  - 描述：L3-B BLOCKING。`ProcessManager` 有归属检查但 job 层没有；同进程其它会话/subagent 可枚举、读取、终止他人作业与输出。`.cmd` 内含整条命令原文且无清扫器。
  - 登记来源：`.dev-team/review-promote-B.json`（2026-11-04）

- [ ] `B017` - P1: 失败轮正文承载只接到次要路径，主失败路径仍丢正文
  - 路径：`crates/pony-agent-core/src/agent/runtime/turn_persist.rs:478`（`fail_stream_turn_with_hook_dispatch`）
  - 描述：P0-3 第二刀给 `emit_stream_failed` 加了 `partial_text`，但 5 个调用点里只有 `turn_stream.rs:2626`（保存附件失败）能直接拿到已渲染正文；工具失败/熔断这类**主失败路径**走 `turn_persist.rs:478`，正文在 per-hop `model_hop_trace_contents` 里，需要给 helper + 3 个调用点加参数（小重构）才能接上。trace `pa_01a1a134` turn-3 正是这类场景。
  - 登记来源：P0-3 第二刀实现回执（2026-11-04）

- [ ] `B018` - P1: provider 多工具调用真正执行（P1-1 完整切片）
  - 路径：`crates/pony-agent-core/src/agent/provider/{mod.rs,openai_sse.rs}`、`runtime/{turn_stream.rs,turn_sync.rs}`
  - 描述：本轮已消除**静默截断**（OpenAI 保留首个 + 结构化告警 `multi_tool_calls_dropped`；Anthropic Err 消息带数量与名字；新增 `extract_*_tool_calls` 复数 pub 接缝）。真正执行 N 个调用仍需独立一波：约 350–550 行 / 4–5 文件，最大风险在**批量回填协议形态**——沿用 1:1 的 `anthropic_assistant_content_for_tool_result` 会让同一 assistant 内容重复 N 次触发 Anthropic 400，必须一次 assistant + N 个 tool_result；Ask 类交互工具须作批终止符。
  - 设计文档：`.dev-team/designs/multi-tool-call-execution.md`
  - 登记来源：W28b 侦察 + 本轮 D 卡裁决（2026-11-04）

## 已完成
- [x] `B015` - P1: 重试指纹保留 timeoutMs 语义（延长=新策略，分桶实现）
  - 路径：`crates/pony-agent-core/src/agent/runtime/stream_support.rs::normalize_run_timeout_argument_value`
  - 结果：`timeoutMs` 由「整体剔除」改为 `floor(log2)` 分桶值写回（同桶同指纹、跨桶异指纹）；防风暴仍由同工具 4 次闸兜底；迁移 3 处原冻结断言。验收 `timeout_signal_promote_test` 4 passed、adversarial 4、harness 12。



- [x] `B013` - P1: Windows 一次性批处理脚本残留（含命令原文）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs`（`<ws>/.tmp/pony-agent-run-{sid}.cmd` 写入与 promote 早返回跳过删除）
  - 描述：promote 路径按红线不删脚本，清理责任移交 job 生命周期但未实现 ⇒ Windows 上每 Run 累积一个含整条命令原文的 `.cmd`（可能含内联凭据）。需在 job 终态回收时删除对应脚本（沿用 B008 的 `release_promoted_entry`），并加 `.tmp` 陈旧文件清扫器。
  - 登记来源：`.dev-team/review-promote-B.json`（2026-11-04）；本机 Linux 无法验证，需 Windows CI 或目标机验证。

## 已完成
  - 结果：已修复（Linux 可验证形态）。`register_promoted` 末尾增 `cleanup_script: Option<PathBuf>`；终态回收点 `release_promoted_entry` 里 `remove_file`，失败只 warn、不改作业状态；promote 分支传 `batch_script_path.clone()`（POSIX 恒 None）。验收 `promote_script_cleanup_test` 3 passed（删除 / None 兼容 / 清理失败不污染状态）。Windows 真机端到端仍未验证（需 CI 或目标机跑一次 promoted run 后检查 `<ws>/.tmp` 无残留）。

- [x] `B009` - P0: promote 后取消失效（用户 Stop 停不下来）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs:3004`（`CommandCancelWatcher` 随函数返回 drop）
  - 描述：L3-A BLOCKING，实测 cancel 后 `sleep 5` 仍跑完 exit 0。需把 turn 取消语义延续到 promoted 作业（或把 promoted 注册进可取消集合）。
  - 结果：已修复。promoted 作业登记 `CancellationToken` + 作业级 watcher（终态即退出）⇒ cancel 后 ≤1s 转 killed；`process.rs` POSIX 侧 spawn 改 `process_group(0)` + `terminate` 先 `killpg` 再 reap ⇒ 消灭孙进程孤儿。验收：`ct_p8`（含 `/proc` 组内无存活正控与分流断言）8 例全绿。

- [x] `B008` - P0: promoted 作业的 `ManagedProcess` 条目永不回收（每 run 泄漏 1 条目 + Windows Job 句柄）
  - 路径：`crates/pony-agent-core/src/agent/process.rs`（poll 不 remove）、`tools.rs` promote 早返回跳过二次 kill、`ProcessManager::shutdown` 零生产调用方
  - 描述：L3-A BLOCKING。promote 后 `legacy_run_session_id()` 每 Run 唯一，`shutdown(session)` 无生产调用方 ⇒ 设计所称「由 shutdown 收口」不可达；promoted 进程只能靠模型 `job_kill`。
  - 登记来源：`.dev-team/review-promote-A.json`（2026-11-04）
  - 结果：已修复（`jobs.rs` `released` 标志 + `release_promoted_entry()`，复用 `kill_if_exists` 的已退出回收；两条 `#[cfg(test)]` 单测断言 `is_tracked=false`；promote 7 / pa105 5 全绿）。

- [x] `B005` - P1: 真·promote：Run 超时把运行中命令转后台 job（DSH 对齐）
  - 路径：`crates/pony-agent-core/src/agent/jobs.rs`（新增 promote 适配层）、`crates/pony-agent-core/src/agent/tools.rs:3011,3122-3176`
  - 描述：侦察结论（.dev-team，2026-11-04）——Run 的进程所有权在 `ProcessManager`（`process.rs:306`）、`poll` 为「取走即清空」（`:869-872`）、`kill_after` 无 disarm API（`:433-443`）、取消 watcher 随函数返回 drop。需在 jobs.rs 内做 `JobKind::Promoted` 适配层 + 生命周期重构；在此之前超时仍为 kill 语义。
  - 登记来源：tool-timeout-policy 波次一（契约 v2 明确不做）
  - 结果（会话归属部分已修）：`JobSession.owner` + `ensure_owned`；新增 `job_{output,kill,list,start}_scoped` 变体（原签名保留为无归属包装，避免破既有集成测试编译），工具路径全部透传 `ToolExecutionContext.session_id`；kill 校验在终止之前、list 直接过滤不泄露存在性。验收 `job_session_scope_test` 2 passed（红相：跨会话读/杀全放行，kill 甚至真的杀掉别人作业）。
  - 未修部分转 `B013`：Windows `.cmd` 残留（含命令原文、无清扫器）。

- [x] `B004` - P1: Run 超时提示指向后台路径（CT-5 契约演进波次）
  - 路径：`crates/pony-agent-core/src/agent/tools.rs`（`classify_timeout_hint`，约 :7961）；
  - 描述：超时 hint 目前只给「缩短命令 / 更大 timeoutMs」，不提已 ModelVisible 的 `job_start`/`job_output`；trace `pa_01a1a134` turn-0 正是超时后闷头重试。改文案会撞冻结用例 `crates/pony-agent-core/tests/wave9_green_acceptance_test.rs:143`（逐字断言默认文案），已完成：契约 v3（CT-5/CT-5a）+ 冻结断言迁移 commit `71b9280` + 实现 commit `7675017`。
  - 登记来源：tool-timeout-policy 波次一（契约 v2 明确延后）

- [x] `B000` - P0: 修复 `npx vitest run` 全量门禁偶发 exit=1（ReferenceError: window is not defined @ src/stores/runtime.ts:904）
  - 路径：`src/lib/runtime/utils.ts` 与 `src/stores/runtime.ts`
  - 描述：通过环境销毁检测中断延迟宏任务回调，收口 scheduleDeferredPersist/cancelDeferredPersist 对 window 的裸解引用。
  - 验收：10 次全量连跑 exit=0 且 Errors 0。
