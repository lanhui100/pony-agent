# Agent Note: goal 生命周期两处脱节（事件死通道 / 删除不清理 sidecar）与终态收尾上下文缺失

Status: implemented

## Problem

目标（goal）域的**领域事实**与它的两个消费者（桌面 UI、模型收尾）都不在同一根线上，形成三个各自成立的缺口：

1. **`goal://` 是死通道**：`src/components/GoalBar.vue` 订阅 `goal://changed` / `goal://round`，
   但全仓 `.rs` 零发射点（`git grep -n "goal://"` 在改动前只有前端与文档命中）。
   实际刷新只靠 sessionId 变化与 turn 结束，于是长跑目标期间 UI 的轮次计数、相位、阻塞原因都不动；
   更糟的是「看起来有推送」——契约 `.dev-team/contract-wave-1.md:53` 与
   `docs/architecture/plan-goal-contract.md` §7 都写着后端会发，读代码的人会以为已经接好。
2. **会话删除不清理 goal sidecar**：goal 以 session id 为唯一键写单文件 sidecar
   （`crates/pony-agent-core/src/agent/goal.rs::SESSION_GOALS` → `goal_store.json`），
   而 `HostControlPlane::delete_session` 只删会话本身。条目永久残留、随删除次数单调增长；
   会话 id 若被复用，新会话会读到上一会话的目标。
3. **终态后模型没有收尾指令**：模型在自己发起的 goal 轮里把目标标成 `completed` / `blocked` 后，
   宿主循环因为续轮读面只命中 `phase == "active" && Armed` 而**硬停**；本轮模型没有收到任何
   「把结论写给用户」的信号，于是常见「标记完成但用户读不到结果」。

## Decision

1. **在领域层发射，而不是在 Tauri 命令层补 emit**（`crates/pony-agent-core/src/agent/goal.rs`）。
   新增 `GoalEvent`（`Changed` / `RoundStarted` / `Cleared`）、`GoalEventObserver` 与
   `install_goal_event_observer`；在 `create_goal` / `update_goal` / `advance_goal_round` /
   `clear_goal` **持久化成功之后、写锁释放之后**调用 `notify_goal_event`（观察者可回调读面）。
   理由：goal 变更有三条来源——Tauri IPC 命令、模型工具（`execute_update_goal`）、
   宿主续轮（`advance_goal_round`）。只挂命令层会让「模型自己推进的目标」对 UI 永久不可见。
   core 不依赖 Tauri：桌面层（`src-tauri/src/goal_events.rs`）安装观察者，把事实映射成线缆载荷。
2. **§7 两个冻结事件零改动，`clear` 用 additive 的 `goal://cleared`**（`{sessionId, goalId}`）。
   `clear` 没有 `GoalData` 快照，复用 `goal://changed` 会让 `phase` 字段说谎；
   `goal://changed` 的字段集合与触发时机严格按 §7 原样实现（`Changed` 在 create/update/advance
   后各发一次，`RoundStarted` 与 `rounds_started += 1` 同一次成功）。
3. **`delete_session` 内 fail-soft 清理 goal**（`control_plane/history_commands.rs`）。
   会话删除已是对用户可见的事实，goal 清理失败**不得**回滚或阻塞它；但失败必须留结构化诊断
   `goal_clear_on_session_delete_failed`（`stage` / `session_id` / `error`），否则 sidecar 泄漏
   又变成静默债务。清理点选在 core 的 `delete_session`（唯一真实实现），不重复写一份。
4. **终态收尾走既有工具回灌通道**（`crates/pony-agent-core/src/agent/goal_wrapup.rs` +
   `tools/orchestration.rs::execute_update_goal`）。`completed` 注入 `<goal_complete>`、
   `blocked` 注入 `<goal_blocked>`（带 `blocked_reason`，缺失时用中性兜底句），
   作为 tool result 的 `wrapup` 键；非终态相位不注入。
   「终态后不再发起新轮」不是新增闸门，而是既有续轮判据的结构性结果：两个宿主循环与 TUI 循环
   每轮重读 `continuation_goal_for_run` / `active_goal_for_session`，终态一律返回 `None`，
   单测直接断言这两个真实读面。

## Alternatives considered

- **删掉前端死订阅（B130 的选项 b）**：成本更低，但长跑目标期间 UI 永远停在「建目标那一刻」，
  且模型工具与宿主续轮改动的目标对 UI 不可见。否决。
- **在 `goal_create` / `goal_update` / `goal_command` 三个 Tauri 命令里各自 emit**：
  覆盖不到模型工具路径（`update_goal` tool 直接调 core）与 graph 续轮（`advance_goal_round`），
  正是最需要即时反馈的两条路径。否决。
- **把 `clear` 折叠成 `goal://changed`（phase=某个新值或空）**：`goal://changed` 的载荷语义是
  「目标当前快照」，`clear` 后没有快照；塞一个假 phase 会让前端与日志都读错。additive 事件更诚实。
- **为收尾文案新开一条上下文注入通道（类似 DSH 的 `deferContext`）**：Pony 没有这一层，
  新建一套意味着 runtime 多一条旁路且要重新证明「下一轮一定看到」。tool result 本身就是
  「模型下一次采样必然看到」的通道，复用它不增加机制。否决。
- **在 Tauri 的 `delete_session` 包装层清理 goal**：core 才是删除的唯一实现（桌面只是转发），
  放包装层会让 TUI/未来调用者继续泄漏。否决。
- **goal 清理失败即让 `delete_session` 返回错误**：会话删除已发生，报错只会让 UI 显示「删除失败」
  而实际已删；改为 fail-soft + 结构化诊断。否决。

## Consequences

- 桌面 GoalBar 在 create / update / advance / clear 后即时刷新；新增的 `goal://cleared`
  属 §7 之外的 additive 事件，需 Lead 决定是否把该行补进 `docs/architecture/plan-goal-contract.md`
  §7（本卡未改动冻结文档）。
- 会话删除后 sidecar 不再残留；清理失败可见（warn 日志），但不影响删除语义。
- 终态 goal 的 tool result 多一个 `wrapup` 字段（additive，既有 `ok` / `tool` / `result` 不变），
  模型在最后一次采样看到收尾要求，宿主的停止语义未变。
- 测试面：`agent::goal::b130_event_tests`（四条写入路径 + 幂等 clear 不广播）、
  `agent::goal_wrapup::tests`、`agent::tools::tests::orchestration::terminal_goal_wrapup_is_injected_into_tool_result_b129`、
  `agent::control_plane::history_commands::b130_tests`（成功清理 + 落盘失败 fail-soft）、
  `src-tauri/src/goal_events.rs`（事件名/载荷形状）、`tests/GoalBar-events.spec.ts`（前端订阅与过滤）。
- 附带修复：`goal_tools_authority_gate_b128` 原先不隔离 goal store，并行 goal 用例会把它推进
  「degraded 拒绝写盘」分支随机假红；现与其它 goal 用例共用测试锁 + 独占 sidecar。
- 复盘（工具链）：本卡期间一次 `rustfmt --edition 2021 <file>` 因 rustfmt 默认递归子模块，
  把 crate 内 163 个文件按新版本默认风格重排（本仓无 `rustfmt.toml`、无 fmt 门禁，提交风格来自更早的
  rustfmt 版本）。全部非本卡改动已按「HEAD + 同版本 rustfmt == 工作区」逐文件核对后回退，
  未保留任何纯格式改动；后续需要全仓格式化时应单独立卡并接受一次大 diff。
