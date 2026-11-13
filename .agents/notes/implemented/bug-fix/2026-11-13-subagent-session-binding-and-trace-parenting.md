# Agent Note: 子代理会话归属与 trace 父子关系修复

Status: implemented

## Problem

2026-11-13 对 TUI 会话 `pa_01a1d464-6e78-7653-b3a7-c96f6f0fb332` 的一次
`subagent` 调用做四路交叉审计（runtime log / sessions.db / Tempo / Langfuse），
功能本身成功（子 turn `10757ms` 完成，输出 `32000` 正确回传），但暴露出五处结构性缺口：

1. **子会话未继承父 workspace**：父请求 Environment `cwd=/home/dm/pony-agent`，
   子代理 prompt 却是 `cwd=/home/dm/pony_agent`（下划线）。子会话以
   `workspace_id: None` 启动，`session_workspace_cwd` 回退到默认 workspace
   （`/home/dm/pony_agent`），而父会话绑定 `ws-pony-agent-1792408932994`
   （`rootPath=/home/dm/pony-agent`）。算术任务无感，任何文件类子任务都会读写错目录。
2. **`subagent_id` 不可追踪**：工具结果返回 `subagent-54f746e8-…`，真实子会话却是
   `subagent-fdaf51df-…`。前者由 `orchestration.rs` 独立生成，后者由
   `subagent_runtime.rs` 生成，两个 id 无任何映射，日志/DB 按返回值检索必然落空。
3. **子 `agent.turn` 未挂在 `tool.call` 下**：trace `7041117b05c5b4b8eed0378151111bc8`
   中两者同挂父 `agent.turn`，是兄弟节点。原因是 `current_traceparent()` 在工具派发时
   只能拿到 turn span，而 `tool.call` span 要等工具结束才 backdate 创建。
4. **`fork_context` 是死参数**：schema 与 `SubagentArgs` 都暴露该参数，运行时始终以
   `history: Vec::new()` 启动，模型按 schema 传 `true` 不会得到任何上下文继承。
5. **两处口径不一致**：无工具子 turn 的 `step-call-tool` 永久 `pending`（看起来像卡死，
   实际是无工具可调）；流式路径 `agent.turn` span 从函数入口起算，而
   `turn_duration_ms` 从更晚的 `turn_started_at` 起算，实测 70.334s vs 34905ms。

## Decision

1. **子会话显式继承父 workspace**：新增 `SubagentRunRequest.workspace_id`，工具层从
   `ToolExecutionContext.workspace_id` 透传；缺失时引擎从共享 `SessionStore` 读取父会话
   快照的 `workspace_id`，再缺失则按 `workspace_root` 在已注册 workspace 中反查。
   最终写入 `TurnInput.workspace_id`，由 `stamp_workspace_id` 在子会话首次持久化时盖章。
2. **一个会话只有一个 id**：会话 id 由编排层预分配并通过 `SubagentRunRequest.session_id`
   下传；`HostTurnEngine` 必须复用该 id，`SubagentResult.subagent_id` 同步路径优先采用
   引擎回传的 `outcome.session_id`（兜底预分配值），后台路径直接返回预分配值。
   既有 Stub 引擎忽略预分配 id 时行为不变（默认方法链保持兼容）。
3. **工具 span 前移为真实父 span**：新增 `otel::begin_tool_call_span`，在工具**执行前**
   创建 `tool.call` span 并压入线程本地父链；`current_traceparent()` 优先读该父链，
   子代理/队友 turn 因此成为 `tool.call` 的真子节点。工具结束后的
   `record_tool_call_span_with_call_id_and_kind` 按 `call_id` **取回同一个 span**（从待记账
   队列移除），补写 status/result/duration/会话归属后结束，不再另建第二个 span。门禁关闭或
   未预创建时逐字回落既有 backdate 路径。接线点：`subagent`（同步子轮）与 `spawn_teammate`
   （派生 + 同步驱动的首轮）。待记账队列有 FIFO 上限 64：取消/异常路径若永不记账，最旧
   span 会被主动 `end` —— 否则它永远不导出，且子 turn 的 parent 指向线路上不存在的 span。
4. **`fork_context` 落地为有界 seed**：`fork_context=true` 时引擎从共享 store 读取父会话
   快照，只保留最后一个 assistant 消息之前的历史（截断未完成回合），作为子 turn 的
   `TurnInput.history`。共享 store 不可用或父会话不存在时回落空历史，不伪造上下文。
5. **口径对齐**：流式路径把 `turn_started_at` 移到 `begin_turn_trace` 同点，span 与
   `turn_duration_ms` 覆盖同一区间；无工具完成的 turn 将 `step-call-tool` 标为
   `skipped`（前端类型同步），不再永久 `pending`。

## Alternatives considered

- **只改 `subagent_id` 返回值映射表**：需要在进程内维护 `generated → real` 映射，进程重启
  即失效，且后台路径返回时真实 id 尚不存在；不如预分配一个 id 贯穿两条路径。
- **子 span 导出后回填 parent**：OTel span 一旦 `end()` 不可变更父子关系，只能重建 span；
  代价高于在派发前创建 `tool.call`。
- **`workspace_id` 缺失时继续回退默认 workspace**：这正是本次事故的根因；回退会让文件类
  子任务静默跑错目录，必须显式继承或按 root 反查，解析失败也应保留可诊断的 `None`。
- **删除 `fork_context` 参数**：可以消除“假承诺”，但 schema 已对外暴露且审计计划明确要求
  “以父已完成回合为 seed”；实现有界继承比删能力更符合既定方向。
- **无工具 turn 把 `step-call-tool` 标 `completed`**：会谎报“调用过工具”，使轨迹面板无法
  区分“没调用”与“调用成功”；新增 `skipped` 是唯一不撒谎的状态。

## Consequences

- 子代理的文件类任务与父会话落在同一工作区；日志/DB 可直接用返回值检索。
- Tempo/Langfuse 中 `tool.call` 覆盖真实执行区间，子 `agent.turn` 挂在其下，慢工具归因准确。
- `fork_context=true` 会复制父历史进子上下文，token 成本随父历史增长；默认 `false` 不变。
- 新增 `SubagentRunRequest` 与 `run_turn_request` 默认方法，既有 `SubagentTurnEngine`
  实现零改动；Stub 引擎不感知新字段。
- 口径对齐的代价：流式 turn 的 `turn_started_at` 上移后，**turn 级** TTFT 回落值
  （仅无 per-call ttft 的 provider 使用）会把 turns 前置开销计入首包延迟；
  per-call `ttft_ms`（C2 契约真值）零变更。取「span 与 duration 同真值」而非
  「TTFT 绝对值最小」，因为同一轮出现两个耗时才是不可能解释的缺陷。
- 规模治理：`otel/mod.rs` 已顶到文件规模基线（5728 行），本变更把三个内联测试模块
  合并拆到 `otel/tests.rs`（生产文件降至 4413 行），并把 `scripts/file-size-baseline.json`
  的该条棘轮下调 —— 测试增长不再推高生产文件的规模债。
- 验收覆盖：`otel/tests.rs::p3_precreated_tool_span_is_consumed_by_call_id`（span 消费与
  父链）、`telemetry.rs::completed_without_tool_marks_call_tool_skipped`、
  `subagent_runtime.rs::fork_seed_keeps_history_up_to_last_assistant_message` /
  `workspace_root_match_is_exact_after_trailing_separator_trim` /
  `child_workspace_id_prefers_explicit_then_inherited_then_root_lookup`。
