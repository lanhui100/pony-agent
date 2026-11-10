# Backlog

## 进行中

暂无

## 待办

- [x] `B041` - P1: Windows CI 四个 write_separate 用例失败（reload 后 turn_trace_history 计数/event_id 不符）
  - 路径：`crates/pony-agent-core/src/agent/{sqlite_session.rs,session/log_sink.rs,session/store.rs}`
  - 现象（CI run 42739179126，Windows）：`node_materialize_uses_full_table_not_top_level_filtered_subset`、
    `session_store_write_separate_hot_path_keeps_trace_rows_across_reload`、
    `session_store_write_separate_preserves_branch_trace_across_switch_and_restore`、
    `session_store_write_separate_repairs_missing_trace_row_on_update`；断言为 `left:3/right:2`（多出一条 trace）
    与 `left:None/right:Some("turn-1:4")`（event_id 丢失）。Linux 全绿。
  - 初判：Windows 临时目录为 8.3 短路径（`C:\Users\RUNNER~1\...`），会话日志根由 `db_path.parent()` 派生、
    而 cwd 分组由 `current_dir()` 派生，短/长路径两种字符串形式可能指向**同目录的两份日志** ⇒ 物化重复或跨分组读空。
    另需复核 0106 `startup_repair_deferred` 在 Windows 的 reasons 分支（日志见 `workspace_bootstrap,history_graph`）。
  - 修复（2026-11-10，已随 CI 验证通过）：Windows `LockFileEx` 是**强制**锁，第一个 store 持日志写者锁时第二个 store reload 读日志被拒 ⇒ 先在 reload 前释放写者（`SessionBackend::release_log_writers_for_test` 测试钩子）。
  - 验收：四个用例在 windows-latest CI 全绿。

- [x] `B042` - P1: Windows CI `jobs::tests::promoted_job_releases_manager_entry_after_natural_exit` 输出竞态
  - 路径：`crates/pony-agent-core/src/agent/jobs.rs`
  - 现象（CI run @8feb744a，Windows）：`jobs.rs:1091` 断言 `回收不得影响已读输出，实际 output=""`；
    用例已有 2s 有界重试仍空。Linux 全绿。
  - 初判：`job_output(wait:true)` 在 Windows 上于「进程已退出、stdout 管道未排空」时返回空，
    随后 manager 条目按终态被回收，重试路径（`offset=0`）拿到空 ⇒ 读不到已产出的 "done"。
  - **根因（已修，CI 验证通过）**：`jobs.rs::poll_until_exited` 轮询时**丢弃**了中间结果，而
    `ProcessManager::poll` 是「取走」语义（每次排空缓冲区）⇒ 若输出被一次 `Running` 的 poll 取走，
    终态 poll 就只能拿到空串。修复：跨 poll **累积** stdout/stderr。同时撤销了一次未被验证的
    `DRAIN_SETTLE_MAX` 500→2000ms 改动（纪律：不用未验证的改动充数）。
  - 验收：该用例 windows-latest 全绿（run 932a1837 通过）。

- [x] `B043` - P1: Windows CI `observability_reporting_fixes_test::followup_stream_pure_tool_call_records_first_token_latency` 无 `turn:completed`
  - 路径：`crates/pony-agent-core/tests/observability_reporting_fixes_test.rs`
  - 现象：`turn:completed event must be emitted` 的 `expect` 失败（windows-latest **连续 5 次**命中，
    实为确定性缺陷而非偶发；Linux 全绿）。
  - **根因（已修，测试 harness）**：假 provider 只读到 `\r\n\r\n` 就回写并丢弃连接 ⇒ 27.8KB 请求体
    （含 system prompt）留在内核未读；服务端 close 时客户端**仍在写正文** ⇒ 首包请求即
    `type=request` + `client error (SendRequest)`（日志物证：**非** `connect`、elapsed 1–2ms、
    body_chars=27856），决策流 5/5 重试后 fallback 到同步决策再 5/5 全败 ⇒ 只剩 `turn:failed`。
  - 修复：`read_http_request` 增预算参数、按 `Content-Length` 读满正文再回写；回写后
    `shutdown(Write)` + 排空入站（干净 EOF）；预设响应改按请求语义（`messages` 中 `role=="tool"`
    条数 0/1/2）选取、与连接次序解耦；失败断言附事件序列 / `turn:failed.error` / 实际请求指纹，
    并加「三轮请求指纹齐全」断言。生产代码零改动。
  - 驳正初判：①「`shutdown`/连接唤醒时序窗」不成立（失败发生在请求发送阶段，与收尾无关）；
    ②「重试吃乱顺序响应」无证据（attempt=1 已失败，从未读到任何响应）——该次序缺陷只是隐患，
    已一并消除。
  - 验收：Linux 干净 worktree 全 15 例 ×3 + 单测 ×10 全 exit 0；windows-latest 同一 commit
    `fc20b81e` 连跑 3 次该用例全 `ok`（run 42768957244 attempt 1/2/3）。

- [ ] `B044` - P1: Windows CI `p0_log_first_durability` 4 例被 Windows 强制文件锁打成红（B043 修复后新暴露）
  - 路径：`crates/pony-agent-core/tests/p0_log_first_durability.rs:195/733/802`
  - 现象（run 42768957244 attempt 1/2/3，windows-latest 三次同签名）：`c4_writable_path_unchanged`、
    `c2_index_failure_still_advances_memory_history`、`c8_stale_epoch_batch_never_reaches_the_session_log`、
    `c9_precheck_failure_does_not_write_log` 全败在「读会话日志/读基线日志」：
    `Os { code: 33, kind: Uncategorized, "another process has locked a portion of the file" }`。
  - 初判：Windows `LockFileEx` 是**强制**锁；用例用 `std::fs::read/read_to_string` 直接读
    `session-logs/**/session.jsonl`，而 store 的日志写者仍持锁 ⇒ 读被拒（Linux 为劝告锁，读得到）。
    与 B041 同族（当年解是用 `SessionBackend::release_log_writers_for_test` 后再 reload）。
  - 说明：该文件在 B043 之前从未被执行到（cargo test 在第一个失败测试二进制处停止），属**既有
    Windows 缺陷**，非 B043 修复引入（本次改动仅一个测试文件）。
  - 验收：windows-latest 该 4 例全绿（读日志前释放写者，或改为断言进程内已物化视图）。
- [x] `B019` - P1: T1 工具面单一真值闭合与外部工具通道（ADR 0065，W1）
  - 路径：`crates/pony-agent-core/src/agent/{tools.rs,dispatcher.rs,governed_executor.rs,provider/mod.rs}`
  - 描述：execute_internal 分派表化、`swap_tool_surface` 原子换入、拆 `is_builtin_tool_definition_set`、修只读模式按名嗅探漏洞、`ToolProvider` + `ExternalToolHandler`。
  - 验收：cargo test -p pony-agent-core 全绿；45 内建工具表面不变量逐字节不变；外部 descriptor 经治理分派可执行。

- [x] `B020` - P1: T2 MCP 通道闭合（ADR 0066，W2，依赖 B019）
  - 路径：`crates/pony-agent-core/src/agent/{mcp_client.rs,mcp_resources.rs,capability_bridge.rs}`、`src-tauri/src/lib.rs`、`docs/observability.yaml`
  - 描述：`.mcp.json` 配置加载（凭据只允许 `${env:VAR}`）+ host 命令 + MCP 工具→外部 descriptor + 错误码分源 + 脱敏。
  - 验收：fixture stdio server 三跳、崩溃→`mcp_server_unavailable`、无残留进程、日志无 secret 字面量。

- [x] `B021` - P1: T3 进程外插件宿主协议 v1 与 hooks 沙箱加固（ADR 0067，W1）
  - 路径：`crates/pony-agent-core/src/agent/plugin_host/`、`hook_commands.rs`
  - 描述：stdio JSON-RPC + `pony-plugin-v1` manifest + 能力默认拒绝 + 超时杀进程组；hooks 增 argv 形态与 env 白名单，legacy 保持可用。
  - 验收：fixture 插件全链路；env 集合精确等于白名单；apiVersion 不符拒绝。

- [x] `B022` - P1: T4 五份契约版本化 + conformance kit + 扩展点归属表（ADR 0068，W2 骨架 / W3 冻结）
  - 路径：`docs/contracts/`、`crates/pony-agent-core/{tests/conformance.rs,src/bin/conformance.rs}`、`docs/architecture/extension-points.md`
  - 描述：五契约 schema + 12 例（6 正 6 负）+ 归属表脚本生成（禁止手写副本）。
  - 验收：`conformance --strict` Exit 0；归属表生成物 drift = 0。
  - 结果：四件同批交付。lib 1295 passed / 0 failed；验收 70 例全绿（tool_surface 19 / plugin_host 11 / mcp_channel 12 / conformance 28）；`cargo check -p pony-agent-core` / `src-tauri cargo check` Exit 0；`npm run ext:check` PASS；observability `check-declaration.sh` / `audit-diff.sh` Exit 0。
  - 顺带修复的真实缺陷：只读模式下外部写工具按名嗅探被放行（现按 descriptor 权限事实判定）；`swap_tool_surface` 曾出现「descriptor 可见但不可执行」；追加外部工具会使内建 45 工具落入 fallback（重复+变序，破 KV-cache 前缀）。
  - 未做（明确非目标）：workflow 编排；T5 可见性与信任面（B023）；远程 streamable-http MCP；hooks 迁移到插件协议。

- [ ] `B023` - P2: T5 外部能力的可见性与信任面（本批不做，ADR 0064 明确后续）
  - 路径：`src/components/settings/`、`src/components/chat/WorkspaceTurnItem.vue`、`src/lib/runtime/`
  - 描述：MCP 服务器管理（增删/启停/健康/最近错误）、授权记录与撤销、通用工具渲染 fallback（`toolIconByCanonicalName` 硬编码 13 条）、错误码用户可读文案，三端一致。
  - 登记来源：ADR 0064 Non-Goals 第 8 条（缺它则 T2 上线即「装了但不知道装了什么」）。

- [ ] `B024` - P2: `canonical_tool_name` 别名归一化收敛到 registry alias index
  - 路径：`crates/pony-agent-core/src/agent/tools.rs:7352`
  - 描述：T1 交付时明确留待后续：名称归一化仍是手写 match，与 registry alias index 并行存在，属潜在第二真值源。
  - 登记来源：ADR 0065 决策 1（单源收尾项）

- [ ] `B025` - P3: `ToolDescriptorSource` 增 `Plugin` 变体并把插件 descriptor 从 `dynamic` 切过去
  - 路径：`crates/pony-agent-core/src/agent/tools.rs:404`、`crates/pony-agent-core/src/agent/plugin_host/mod.rs`
  - 描述：当前插件工具挂 `Dynamic` + `descriptor_id=dynamic:plugin:<name>:<tool>`，来源枚举实为 4 值；新增 `Plugin` 为 additive，落地后契约 schema 枚举与归属表需重生（`--write`）。
  - 登记来源：ADR 0067 偏差 1 / T4 复核

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

## 已完成

- [x] `B018` - P1: provider 多工具调用真正执行（P1-1 完整切片）
  - 路径：`crates/pony-agent-core/src/agent/provider/{mod.rs,openai_sse.rs}`、`runtime/{turn_stream.rs,turn_sync.rs}`
  - 描述：本轮已消除**静默截断**（OpenAI 保留首个 + 结构化告警 `multi_tool_calls_dropped`；Anthropic Err 消息带数量与名字；新增 `extract_*_tool_calls` 复数 pub 接缝）。真正执行 N 个调用仍需独立一波：约 350–550 行 / 4–5 文件，最大风险在**批量回填协议形态**——沿用 1:1 的 `anthropic_assistant_content_for_tool_result` 会让同一 assistant 内容重复 N 次触发 Anthropic 400，必须一次 assistant + N 个 tool_result；Ask 类交互工具须作批终止符。
  - 设计文档：`.dev-team/designs/multi-tool-call-execution.md`
  - 登记来源：W28b 侦察 + 本轮 D 卡裁决（2026-11-04）
  - **进展（2026-11-04）**：已消除静默截断（`extract_*_tool_calls` 复数接缝 + OpenAI 丢弃告警 + Anthropic 诊断）；
    D1b 已交付两个批量回填纯函数 `openai_messages_with_tool_results` / `anthropic_messages_with_tool_results`
    （1 assistant + N tool / 1 assistant + 1 user(N tool_result)，含「assistant 恰 1 条」负向护栏与 N=1 等价 legacy 的单测）。
  - **剩余（须新会话 + L3 复核）**：D1a（`6d743b82`）与 D2（`8f33eb27`）已完成；剩余 D3：`turn_sync.rs` 循环批执行（D3a）→ `turn_stream.rs::handle_stream_tool_turn` 批执行（D3b，产品主路径）→ 之后删除 `ensure_single_anthropic_tool_call` 的 >1 Err 并翻转其单测。详见设计文档 §7。原估 350-550 行经实测修正为 **900-1500 行 / 10-13 文件**。
  - 结果（2026-11-04 收口）：D1b `a57f636b`（批量回填纯函数）→ D1a `6d743b82`（tool_calls 复数位）→ D2 `8f33eb27`（provider 批量 followup）→ D3a `c3704fab`（turn_sync 批执行）→ D3b `efbb9195`（turn_stream 批执行 + 删除 Anthropic >1 Err）→ D4（parallel_tools 并发接线，开关 `PONY_AGENT_PARALLEL_TOOLS` 默认关）。`--lib` 1244/0。
  - 遗留（另开卡）：Anthropic/Responses 流式解析仍单值；N>1 followup 非真增量；批内仅首个过 normalize_tool_directive；non-pending 交互工具停批后余下丢弃。
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

- [ ] `B026` - P1: 抽 `HostControlPlane` 门面 + transport 抽象（HTTP/WS 服务化前置）
  - 路径：`crates/pony-agent-core/src/agent/control_plane/`、`src-tauri/src/lib.rs`、`src-tauri/src/tauri_adapter.rs`
  - 描述：`HostControlPlane` 现为 struct 而非 trait，94 个 `#[tauri::command]` 与 `generate_handler!` 宏绑死，`TurnEventSink` 是唯一宿主注入点但无 transport 层（SSE 只有内存 formatter，`headless.rs` 的 sink 无 writer 抽象）。新增 HTTP 端需复制 94 个命令签名 + 16 个前端 listen + 整套指标派生层。
  - 验收：非 Tauri 宿主（HTTP/WS）可在不复制命令壳的前提下复用同一 core 命令面与事件流；`non_tauri_harness` 扩展为正例；Tauri 侧行为零变更。
  - 登记来源：wave-metrics 架构审计（`arch-audit` 2026-11-06）；本轮明确不做（用户交付边界：P2 独立立项，不夹带重构）。

- [ ] `B027` - P1: 成本协议口径统一（同一 turn 两个 costUsd）
  - 路径：`crates/pony-agent-core/src/agent/turn_flow.rs:2672`、`crates/pony-agent-core/src/agent/runtime/turn_persist.rs:67`、`crates/pony-agent-core/src/agent/budget.rs:306`
  - 描述：wire `costUsd` 走协议感知 `estimate_cost_with_protocol`，落库 `TurnTraceRecord` 用固定 OpenAI 口径的 `estimate_cost`；`budget.rs` 本地算 `uncached=input-cached` 却调固定 OpenAI 的 `calculate_cost_with_pricing`（anthropic 用错协议），且 `total_input_tokens += uncached` 与 `total_tokens += input+output` 分项和≠total。
  - 验收：同一 turn 的 wire/落库/budget 三处成本逐位一致；anthropic 协议回归用例（uncached 不减 cache_read）。
  - 登记来源：wave-metrics 指标链路审计（`metrics-audit` 2026-11-06）。

- [ ] `B028` - P2: compaction 的指标语义标记（`context_reset` / basis）
  - 路径：`crates/pony-agent-core/src/agent/runtime/compaction_ops.rs:179`、`crates/pony-agent-core/src/bin/tui.rs`、`src/stores/runtime.ts`
  - 描述：`/compact` 用 `replace_session_history` 把历史换成摘要，下一轮 prompt 真实变小，但 UI 只追加一条 System 消息、不标注也不重置指标 ⇒ 用户看到「上下文无故变小」。需在压缩事件上携带标记并让快照带 `basis=current_context|peak`。
  - 验收：压缩后指标区显式标注「已压缩」，且上下文语义可自解释。
  - 登记来源：wave-metrics 指标链路审计；本轮快照已声明 `contextRatio` 语义（当前调用上下文/窗口），但未做压缩标记。

- [ ] `B029` - P2: wave-metrics L3-A 剩余 ADVISORY 债务
  - 路径：`crates/pony-agent-core/src/bin/tui.rs:3590`、`src/stores/runtime.ts:4308`、`src/components/HomeStatusPanel.vue:137`、`:73`
  - 描述：① `clear_messages()`（换 session 的另一入口，生产链路未调用）不清 `live_metrics`；② 桌面 store 6 处快照落库点位于 turnId 门禁与事件游标去重之前，且无 step 单调守卫（迟到/乱序事件可自愈型回写）；③ `HomeStatusPanel.vue` 圆环仍自算百分比，与快照 `contextRatio` 双公式并存且 `contextPercent` 无生产消费方；④ `≈` 估算标记改由快照 origin 驱动，却标注在 trace 会话累计（Σ output/cache）上。
  - 验收：四项任一收敛后，TUI/桌面均只保留单一口径；②需补乱序事件回归用例。
  - 登记来源：`.dev-team/review-join-wave-metrics-a.json`（L3-A 第 1 轮，ADVISORY 2/3/5/6）

- [ ] `B030` - P1: 指标快照的首跳窗口与持久化（消除「live 与重放两套口径」）
  - 路径：`crates/pony-agent-core/src/agent/runtime/turn_stream.rs`、`crates/pony-agent-core/src/agent/session/types.rs`（`TurnTraceRecord`）、`src/lib/runtime/useTraceProjection.ts`
  - 描述：① `provider_call_records` 仅在调用完成后 push ⇒ 每轮**首个调用**的整个流式期无快照，回退 legacy 渲染（含 `clamp(0,100)`），轮末一次性跳变；② 快照不落 `TurnTraceRecord` ⇒ 重启/resume/重放由 trace 反推第二公式，同一轮 live 与重放可能给出不同值（实测口径差 92% vs 78%）。
  - 验收：首跳流式期可产出「进行中」口径（或显式声明未知）；重启后同一轮指标与 live 逐字段一致。
  - 登记来源：`.dev-team/review-plan-wave-metrics.json` BLOCKING-1/2（契约 §7.1 R1/R2）

- [ ] `B031` - P2: 指标线缆的自描述与能力标记
  - 路径：`crates/pony-agent-core/src/agent/runtime/types.rs`、`crates/pony-agent-core/src/agent/metrics_snapshot.rs`、`src/types/runtime.ts`
  - 描述：① `skip_serializing_if` 使「旧核无此能力」与「新核本帧无快照」在线缆上同形，新端无法做能力探测；② `cacheHitRatio`/`contextRatio` 为 0..1 分数但键名不自描述，而同仓 `HomeStatusPanel` 同名 prop 装的是百分数字符串，新端易误读；③ `totalTokens` 名为 total 实为末次调用值（作用域混装，UI 无「本次/本轮」限定词）。
  - 建议：`metrics` 增 `schemaVersion` 或复用 `eventVersion` 做能力标记；比率键名或注释固化单位；UI 文案补作用域限定。
  - 登记来源：契约 §7.1 R4/R5；方案对抗审查 ADVISORY Q1a/Q4b/Q5

- [ ] `B032` - P2: 事件帧体积预算纳入 CI
  - 路径：`crates/pony-agent-core/tests/ts_contract_snapshot.rs` 或新增体积断言、`.dev-team/nfr-baseline.json`
  - 描述：本波实测 `TurnStreamEvent` 517 B/帧基线，3-record 帧 +36.5%（快照 22 字段）。已入 NFR 预算 700 B/帧但**无机器门禁**，后续再加字段会静默膨胀（IPC/SSE 带宽与前端解析成本）。
  - 验收：CI 对代表性事件帧做体积断言，超预算即红。
  - 登记来源：契约 §7.2；方案对抗审查 ADVISORY Q6

- [ ] `B033` - P2: cache 写入（`cache_creation_input_tokens`）计价维度缺失
  - 路径：`crates/pony-agent-core/src/agent/cost.rs`（`ModelPricing` / `PRICING_TABLE`）、`crates/pony-agent-core/src/agent/telemetry.rs`（`ProviderCallCacheRecord`）
  - 描述：`ModelPricing` 只有 `input_per_mtok` / `output_per_mtok` / `cache_read_per_mtok`，**无 cache write 单价**；`calculate_cost_with_protocol` 完全不读 `TokenUsage.cache_creation_input_tokens` ⇒ anthropic 的缓存写入成本当前**完全未计价**（Anthropic 缓存写入通常为 input 的 1.25×，属系统性低估）。`ProviderCallCacheRecord` 亦无该字段，故 per-call 成本（含 `TurnMetricsSnapshot.costUsd`）无法纳入。
  - 验收：价格表增 cache-write 维度；anthropic 单次调用的 cache 写入费可被计价并与官方账单口径对齐；`ProviderCallCacheRecord` 增字段（wire additive）并贯通快照。
  - 登记来源：B027 实施期识别的**不同根因**缺口（口径统一 ≠ 计价维度补全），见 `.dev-team/contract-matrix-cost-protocol.md` §6。

- [ ] `B034` - P2: 治理汇总前端永不拉取（`fetchUsageGovernanceSummary` 零调用点）⇒ UI 数据面永远走 trace fallback
  - 路径：`src/stores/runtime.ts:844`、`src-tauri/src/lib.rs:715-720`、`crates/pony-agent-core/src/agent/usage_governance.rs:437-480`
  - 描述：后端有权威治理汇总（读 `usage_ledger` SQL），命令也已暴露，但前端 action 无任何调用点 ⇒ 前端 `usageGovernanceSummary` 恒为本地兜底空壳，与后端权威值可能长期背离。属 B027 同类「双口径」缺陷（一个是成本，一个是治理汇总）。
  - 验收：UI 改走后端权威汇总（或明确删除前端兜底，二选一），并补一条一致性用例。
  - 登记来源：B027 侦察（E2 explorer，2026-11-06）；**非本波修复**（超出 B027 写域与验收面）。

- [ ] `B035` - P3: 预算熔断阈值语义不一致（`>` vs `>=`）
  - 路径：`crates/pony-agent-core/src/agent/budget.rs:337`、`crates/pony-agent-core/src/agent/cost.rs:444-458`
  - 描述：`CostTracker` 用 `total_cost_usd > limit` 才熔断，`cost::is_budget_exceeded` 用 `>=` ⇒ 「恰好等于上限」时两处判定相反。`CostTracker` 当前零生产调用方，故影响仅在启用时暴露。
  - 验收：二选一并写进契约（建议 `>=`，与「达到上限即不可再用」的产品语义一致），补边界用例。
  - 登记来源：B027 侦察（E2 explorer，2026-11-06）。

- [ ] `B036` - P1: 成本聚合粒度统一（同帧 `metrics.costUsd` ≠ `trace/wire costUsd`）
  - 路径：`crates/pony-agent-core/src/agent/metrics_snapshot.rs:87-130`（逐调用 Σ + countable 过滤）、`crates/pony-agent-core/src/agent/turn_flow.rs:2711`（轮级聚合一次计价）、`crates/pony-agent-core/src/agent/otel/mod.rs`（span 逐调用，缺 usage 记 `unwrap_or(0)`）
  - 描述：同一帧存在三种成本聚合粒度：① wire/trace/ledger = **轮级聚合 usage 一次计价**（B027 已统一到同函数）；② `TurnMetricsSnapshot.costUsd` = **逐调用计价后求和**且过滤不可计数记录；③ `llm.chat` span = 逐调用且缺 usage 按 0 计。三者协议口径已一致，但**聚合粒度不同**，在「逐调用 6 位取整误差」「output=0 记录被过滤」「openai cache>input 饱和」等场景会给出不同数值。
  - 机器反例（L3-B 实测）：A 0.000040 vs 0.000039（取整累积）；B output=0 被过滤 0.1575 vs 0.3075（差 ~2×）；C openai cache>input 0.004125 vs 0.003125。
  - 验收：三处择一为唯一口径（建议轮级聚合为「账单真值」、逐调用为「诊断明细」并在字段名/文档显式区分），补三条反例回归用例；span 缺 usage 改为不记 `$0`（与 C5「未知不伪造」对齐）。
  - 登记来源：`.dev-team/review-cost-l3a.json` ADVISORY 1/2、`.dev-team/review-cost-l3b.json` ADVISORY 1（2026-11-06）。

- [ ] `B037` - P3: 测试诊断噪声与隔离基线（GATE-C 副产物）
  - 路径：`vitest.config.ts`、`tests/update-signed-store.spec.ts`、`src/stores/__tests__/plan-goal-refresh.spec.ts`
  - 描述：全量运行时 `console.warn(error)` 会把**创建栈**打到 stderr，`❯ tests/xxx.spec.ts:118:62` 这类帧极易被误读成「该文件失败」（本波即因此误判两个从未失败的文件）；建议 ① 在 `vitest.config.ts` 显式声明 `isolate/clearMocks/restoreMocks/unstubGlobals` 抗污染基线；② 对预期告警用 `vi.spyOn(console,'warn')` 静音并**显式断言**（只增不减断言强度）。已在 GATE-C 中验证可行（4 次全量：3 次 149/149 绿、1 次仅 T7 环境性红），但因**发版冻结期不引入全量 mock 语义变更**而回退。
  - 验收：全量 149/149 稳定；两个 spec 断言数不减且新增告警断言。
  - 登记来源：task-8 执行结论（2026-11-06）。

- [ ] `B038` - P2: `wave47-ob2-enforcement` T7 依赖实时工作树，脏树即红
  - 路径：`tests/acceptance/wave47-ob2-enforcement.spec.ts:223`
  - 描述：T7 调用 `ponygo observe audit --level ob2`，该命令扫描**实时工作树变更集**（实测「扫描 N 个变更文件」）⇒ 本地有未提交改动（尤其 `crates/**`、`scripts/**`）时会翻红；且经 `execSync` 默认 60s 上限，机器负载高时可能超时。CI 在干净树 + 提交后运行，故不阻塞发版；但本地/并发开发期会制造假红。
  - 验收：T7 对「工作树脏」有明确判定（要么固定到 base commit 的 diff，要么在脏树下跳过并标注），并把超时显式抬到与实测匹配的值。
  - 登记来源：task-8 执行结论（2026-11-06），本波实测 run2 唯一失败项。

- [ ] `B039` - P2: 快照 `first_token_latency_ms` 取「末次 hop 的 TTFT」，不是本轮首包
  - 路径：`crates/pony-agent-core/src/agent/metrics_snapshot.rs:191`（`first_token_latency_ms: last.first_token_latency_ms`）、消费方 `src/components/HomeStatusPanel.vue` / `crates/pony-agent-core/src/bin/tui.rs`
  - 描述：`TurnMetricsSnapshot.first_token_latency_ms` 直接透传**最后一次** per-call 记录的首包时延，多 hop 轮次里该值与本轮「用户等到首字」的时间无关（远端轨迹首包 5470ms，快照显示 4340ms，后者是末次 hop 的值）。ADR 0073 已明确把它列为相邻但独立缺陷、不在速度口径内处理。
  - 验收：快照首 token 取「本轮第一个到达 delta 的 hop 的 TTFT」（无则 `None`），桌面端 / TUI / 未来 HTTP 面同源；补多 hop 断言（首 hop 早期 + 末 hop 晚期可区分）。
  - 登记来源：ADR 0073 `## 影响` 末条；诊断见 `.dev-team/contract-matrix-token-speed-v2.md`。

- [x] `B040` - P2: 插件子进程 env 白名单键名被强制大写，小写代理键白名单语义失效
  - 路径：`crates/pony-agent-core/src/agent/plugin_host/capability.rs:156`（`OsString::from(name.to_ascii_uppercase())`）、`crates/pony-agent-core/src/agent/plugin_host/capability.rs:65`；用例 `crates/pony-agent-core/tests/plugin_host_contract.rs:365`
  - 描述：`child_env()` 对白名单变量名统一 `to_ascii_uppercase()` 落库，而 Linux 上 `ENV_ALLOWLIST` 含小写代理键（`http_proxy` / `https_proxy` / `no_proxy`）⇒ 子进程实际收到 `HTTP_PROXY` 等大写键；`t3_plugin_child_env_is_exactly_the_allowlist` 实测 `left={HOME,HTTPS_PROXY,HTTP_PROXY,LANG,NO_PROXY,PATH}` vs `right=…+{http_proxy,https_proxy,no_proxy}`（`cargo test -p pony-agent-core --test plugin_host_contract` exit 101，10 passed / 1 failed）。大小写敏感平台上「白名单精确等于」的契约因此不成立，且小写变量丢失可能让插件绕过/失去代理配置。
  - 验收：白名单按平台语义保留原始键名（大小写敏感平台原样透传，Windows 保留大小写不敏感行为）；该用例转绿；顺带确认 `is_proxy_env_name` 对小写键的 userinfo 脱敏仍然生效。
  - 登记来源：本轮 gate 实测（2026-11-06），与 token 速度口径无共享代码路径。
  - 完成（2026-11-08）：白名单键名按平台语义原样透传（与 MCP 侧 F8a 对齐），
    `plugin_host::capability` 8 passed、`plugin_host_contract` 11 passed（t3 转绿）。
    决策记录：`.agents/notes/implemented/bug-fix/2026-11-08-plugin-env-allowlist-key-case.md`。
