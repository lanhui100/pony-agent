# goal 能力事实核查 & 与 DSH `packages/goal` 差距分析

- 日期：2026-11-12
- 波次：goal-1（`/goal` 斜杠命令 + `clear` 动作）
- 契约真源：`.dev-team/contract-matrix-goal-command.md`（C-1..C-7，FROZEN）
- 对照对象：`/home/dm/deepseek-harness/packages/goal`（`goal` / `command-goal` / `goal-round-driver` / `tool-goal`）
- 取证口径：所有「本仓」结论均给 `文件:行号`；所有「DSH」结论均给 `包/文件`。未验证项显式标注「未取证」。

---

## 1. 本仓 goal 能力现状（事实核查）

### 1.1 持久化与状态机（Rust core，唯一真值源）

| 能力 | 落点（取证） | 说明 |
| --- | --- | --- |
| 目标数据结构 | `crates/pony-agent-core/src/agent/goal.rs:29` `GoalData` | 含 `phase` / `activation` / `revision` / `rounds_started` / `max_goal_rounds` / `blocked_reason` |
| sidecar 存储路径 | `goal.rs:97` `goal_store_path()`，覆盖点 `goal.rs:9` `PONY_AGENT_GOAL_STORE_PATH` | 单 JSON sidecar，非会话事件日志 |
| 创建 | `goal.rs:559` `create_goal` | 无「重复创建」硬拦（可覆盖 completed） |
| 读取 | `goal.rs:595` `get_goal` | — |
| 更新 + CAS | `goal.rs:605` `update_goal` | `UpdateGoalAction = complete/pause/resume/edit/blocked`；`revision` 乐观锁 |
| 清除（本波新增） | `goal.rs:686` `clear_goal` | 删记录 + 原子 `persist_goal_store`；落盘失败回滚内存；无 tombstone |
| 续轮真值源 | `goal.rs:297` `active_goal_for_session` | 仅 `phase=active` 且 `activation=armed` 且 session 精确匹配才命中 |
| 轮次推进 | `goal.rs:361` `advance_goal_round`、`goal.rs:525` `advance_goal_round_for_outcome` | 预算耗尽/失败熔断经 `goal.rs:498` `goal_round_stop_reason` |
| 失败熔断 | `goal.rs:404` `MAX_CONSECUTIVE_GOAL_ROUND_FAILURES = 2` | 连续失败即停，带 stop reason |
| 续轮提示模板 | `goal.rs:313` `build_goal_round_message`、`goal.rs:325` `is_goal_round_prompt` | 固定模板（契约 wave1 已冻结） |
| 单飞租约 | `crates/pony-agent-core/src/agent/goal_lease.rs:35` `try_acquire` | 每会话一把 `GoalRoundLoopLease`（`:28`），防并发续轮 |

### 1.2 人类命令面（本波新增）

| 能力 | 落点 | 说明 |
| --- | --- | --- |
| 语法解析 | `crates/pony-agent-core/src/agent/goal_command.rs:50` `parse_goal_command` | 大小写不敏感、首 token 切分；`show/create/edit/pause/resume/complete/clear` |
| 状态机执行 | `goal_command.rs:83` `execute_goal_command` | 文案与相位建议均在 core；`clear` 后 `active_goal_for_session` 立即 `None` |
| 用法真源 | `goal_command.rs:15` `GOAL_COMMAND_USAGE` | 桌面 / TUI 共用，禁止两端各写一份 |
| Tauri 接入面 | `src-tauri/src/lib.rs:468` `goal_command`（注册于 `:1535`） | 返回 `{action, message, goal}`（camelCase） |
| 桌面分发 | `src/lib/chat/slash-commands.ts` `goalCommandArgs` / `describeGoalOutcome`；`src/components/HomeWorkspace.vue`（`executeSlashCommand` 内 `command === "goal"` 分支） | 命中即清草稿、不透传模型；回执走 `/compact` 同一条 info 通道（单行折叠） |
| TUI 分发 | `crates/pony-agent-core/src/bin/tui/goal_command.rs`（`goal_command_args` / `handle_goal_command`）、`bin/tui.rs` 命令表 `"/goal"` 与分发点、`bin/tui/queue.rs` `is_local_control_command` | 带参仍算命令；流式期就地执行（不拆在飞轮次、不入队） |

### 1.3 续轮驱动与 UI

| 能力 | 落点 | 说明 |
| --- | --- | --- |
| 规划器续行判定 | `crates/pony-agent-core/src/agent/planner.rs:97-104`、`decide_for_armed_goal` | 优先级 1 = armed goal（唯一真值源），优先级 2 = `run.goal` + 关键词 |
| 续轮提示注入 | `crates/pony-agent-core/src/agent/context.rs:1160`（`active_goal_for_session`）、`goal.rs:313` | 宿主循环按预算与熔断推进 |
| 桌面目标条 | `src/stores/goal.ts`（`fetchGoal`/`createGoal`/`updateGoal`，`goal_get|goal_create|goal_update`）、`src/components/GoalBar.vue` | `/goal` 执行后 `fetchGoal` 刷新，快照以后端为准 |
| TUI 目标横幅 | `bin/tui.rs:1402` `goal_state`、`:2457` `set_goal`、本波新增 `clear_goal_banner`；`bin/tui/projection.rs:206` `tui_goal_from_goal` | `clear` 后横幅立即消失 |

**结论**：本仓 goal 域在「持久化 + CAS + 单飞租约 + 续轮驱动 + 双端可视化」上完整；本波补齐了唯一缺口——**人类命令面**与**清除动作**。

---

## 2. 与 DSH `packages/goal` 的逐项差异

| 维度 | 本仓 | DSH | 差异性质 |
| --- | --- | --- | --- |
| 持久化模型 | 快照 sidecar（`goal.rs` 单 JSON）+ 进程内 HashMap + `revision` CAS | append-only 会话事件 `goal/change` + 纯 `fold`（`goal/src/fold.ts`、`goal/src/domain.ts`） | **结构性**：DSH 可重放/可审计，本仓只看当前快照 |
| 清除语义 | 物理删除记录（`goal.rs:686`） | 快照变更 + **清除墓碑** `GoalClearChangeMeta{cleared, clearedAt}`（`goal/src/domain.ts`） | DSH 保留「清过什么」的历史；本仓清完即无痕 |
| 相位取值 | `active/paused/blocked/completed`（`goal.rs:29`） | `active/paused/blocked/complete`（`goal/src/types.ts`） | 命名差异（`completed` vs `complete`），语义一致 |
| 激活态存储 | `activation` 落 `GoalData` 并持久化（armed/disarmed） | 激活态是**进程内**、与持久相位分离（`goal/src/types.ts` 注释） | 本仓把「是否武装」写盘；DSH 明确不写盘，重启即 disarm |
| 命令动词集合 | `show/create/edit/pause/resume/complete/clear`（`goal_command.rs:50`） | `show/create/edit/pause/resume/clear`（`command-goal/src/index.ts`，**无 complete**） | 本仓多一个 `complete` 动词；DSH 的完成由模型工具轮次完成 |
| 命令产物 | `GoalCommandOutcome{action,message,goal}`，渲染边界在两端 | `CommandResult{kind:'success',text}` + `createUserMessage` 注入会话 | DSH 把命令回执与人消息同源；本仓明确「不入会话历史」 |
| 相位提示 | `phase_commands`（`goal_command.rs:262`） | `commandHint` 额外区分 `activation`（armed ⇒ 建议 pause / disarmed ⇒ 建议 resume） | DSH 提示更贴当前可行动作；本仓提示未编号激活态 |
| 权限/权威模型 | 无：`create_goal`/`update_goal` 工具对模型完全开放 | `tool-goal/src/authority.ts`：`GoalToolAuthority = direct-human \| goal-round`，要求「精确活着的调用 agent + 打开的模型轮次事件切面」，错误码 `GOAL_TOOL_AUTHORITY_REQUIRED` / `AGENT_REQUIRED` / `DRIVER_REQUIRED` | **结构性**：DSH 防「非目标轮次静默改写目标」，本仓靠人类命令面的冲突提示兜底 |
| 收尾（wrapup） | 无 | `tool-goal/src/wrapup.ts` `renderWrapupContext`：`complete`/`blocked` 后注入结语指令，禁止再调工具 | DSH 保证自主运行结束时模型必对用户交代一次 |
| 轮次提示内容 | 固定模板（`goal.rs:313`，wave1 契约冻结） | `goal-round-driver/src/prompt.ts` `<goal_round>`：Objective JSON 转义、`Round n/max`、grounding/verify/mark-complete 指令 | DSH 提示显式要求「以工作区与工具结果为准、验证后再声称完成」 |
| 轮次调度 | 宿主循环 + 每会话租约（`goal_lease.rs:35`）+ 失败熔断 2 次 | `goal-round-driver/src/index.ts`：`RoundAttempt{queued/claimed/admitted}`、按 `goalId+revision+round` 身份校验、per-agent 串行、静默期收口 | 两者都做单飞；DSH 的身份校验粒度更细（revision 级） |
| 消息归因 | 无显式来源标记（靠 `is_goal_round_prompt` 文本识别，`goal.rs:325`） | `GoalMessageSource{kind:'goal',goalId,revision,round}` 一等公民（`domain.ts`） | 结构性：DSH 可按来源审计/过滤，本仓靠文本指纹 |
| 错误契约 | `Result<_, String>` | `GoalError` / `HarnessError(code)` 稳定错误码 | 本仓无机器可路由错误码 |
| 客户端同步 | 前端 `fetchGoal` 主动拉取（`src/stores/goal.ts`） | `goal/src/client.ts` 投影 + 事件驱动 | 本仓为拉模型，多一次往返 |

---

## 3. 分级优化建议（含是否采纳）

### P0（建议下一波落地）

1. **goal 变更工具的权威校验**（对位 `tool-goal/authority`）
   - 现状风险：`create_goal`/`update_goal` 对模型全天候开放（`goal.rs:559/605`），模型可在**非目标轮次**静默覆盖用户目标；人类命令面已用「已存在目标 ⇒ Err」兜底，但工具面没有。
   - 建议：在 core 增加 `goal_tool_authority()` 判定，仅允许「直接人类轮次」或「当前 session 的 armed goal round」改写目标，其余返回结构化拒绝码。
   - 落点：`crates/pony-agent-core/src/agent/goal.rs`（新增判定）＋ 模型工具注册处（`src-tauri`/工具注册表）。
   - **采纳**：是。属于正确性而非风格问题，且改动局部（不改 sidecar 格式）。

2. **稳定错误码**（对位 `GoalError` / `HarnessError`）
   - 现状：`Result<_, String>`，前端/TUI 只能透传中文文案，无法分支处理。
   - 建议：新增 `GoalErrorCode`（`authority_required` / `revision_conflict` / `not_found` / `invalid_syntax`），保留中文文案为展示层。
   - 落点：`goal.rs`、`goal_command.rs`；`src/types/goal.ts` 同步。
   - **采纳**：是（P0 次优先）。注意 C-0 已声明本轮不引入，故排下一波。

### P1（建议评估后落地）

3. **wrapup 收尾指令**（对位 `tool-goal/wrapup`）
   - 现状：`complete`/`blocked` 后宿主直接停，模型未必给用户一段结语。
   - 建议：在 `advance_goal_round_for_outcome` 判定终态后注入一次收尾指令（复用现有续轮注入通道）。
   - 落点：`goal.rs`（模板）+ `context.rs`（注入点）。
   - **采纳**：是（有真实体验收益），但需与 `build_goal_round_message` 冻结契约解耦，另立模板常量。

4. **命令相位提示纳入 activation**（对位 `commandHint`）
   - 现状：`phase_commands`（`goal_command.rs:262`）对 active 一律给 `pause`，未区分 disarmed 时应建议 `resume`。
   - 落点：`goal_command.rs`（纯文案函数）。
   - **采纳**：是（低成本高一致性），但会改动已冻结文案 ⇒ 需新契约条目覆盖。

5. **目标同步改事件推送**（对位 `goal/src/client.ts`）
   - 现状：`/goal` 后 `fetchGoal` 拉一次；其他端变更（模型工具改目标）桌面不感知。
   - 建议：goal 变更后经 Tauri 事件广播，前端订阅刷新。
   - 落点：`src-tauri/src/lib.rs`（emit）+ `src/stores/goal.ts`（listen）。
   - **采纳**：是（P1），需先有 observability 归属登记。

### P2（记录不落地）

6. **append-only 事件 + fold 重构**：收益是可回放/可审计/天然对账，成本是重写存储层与全部读取路径。**不采纳**（当前 CAS 快照 + 原子 sidecar + 单据幂等已满足本仓规模；引入 fold 会与 `GoalData` 序列化契约、前端类型、TUI 投影全面冲突）。
7. **清除墓碑（tombstone）审计**：**部分采纳**——先在 backlog 登记为观察项；若后续出现「目标被谁清掉」的追责需求，再补最小 `cleared_at/cleared_by` 记录，不做完整事件流。
8. **相位更名 `complete`**：**不采纳**（跨 core/TS/TUI/契约四处破坏兼容，收益仅命名一致）。
9. **续轮提示模板对齐 DSH 的 grounding/verify 文案**：**不采纳本波**（wave-1 契约已冻结模板；如需变更另立契约）。
10. **消息归因一等公民**：**暂不采纳**（当前 `is_goal_round_prompt` 文本指纹可用；升级需改会话消息 schema）。

---

## 4. 验收与证据索引

- core：`cargo test -p pony-agent-core --lib goal`（25 passed）、`cargo check -p pony-agent-core --all-targets`（exit 0）
- Tauri：`cargo check -p pony-agent`（exit 0）
- TUI：`cargo test -p pony-agent-core --bin pony`（128 passed）、`cargo test -p pony-agent-core --test tui_streaming_input_acceptance`（151 passed）
- 桌面：`npm test`、`npm run typecheck`（见交付回执）
- 契约：`.dev-team/contract-matrix-goal-command.md` §3 流水
