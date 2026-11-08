# Agent Team 链路诊断（2026-11-08）

> 数据源：TUI 上传 trace → Langfuse ClickHouse（`kubectl -n observability exec clickhouse-0`，只读 SELECT）+ 当前会话 live 实测。
> 取证副本：`/tmp/team-trace-diag/{pa01b66f-team,devtui-side}.jsonl`。

## 结论

**分派 / 添加任务 / 更新 / teammate 领取 / 回复消息：全部正常。** 发现 1 个真实缺陷（D1）与 1 个使用陷阱（D2），均未改代码。

## 1. Trace 证据（dev team 会话）

- 会话 `pa_01a1b66f`（2026-11-07，Lead + dev-tui）：`team_task_create` 建 2 任务 → `send_message` 派发（delivered=true, driving）→ dev-tui claim/complete，两任务终态 `completed, owner=dev-tui, rev=3`。
- 嵌套团队会话 `subagent-d03ce2fc`（implementer + verifier）：create（含 blocked_by 依赖）/ claim（rev1→2）/ complete（rev2→3）/ CAS 接管（expected_revision=2）全部成功。

## 2. 缺陷与陷阱

### D1（真实缺陷）send_message target 大小写敏感

- Trace 证据：`2026-11-07 15:46:18` verifier `send_message(target="Lead")` → `send_failed: Teammate target not found: Lead`；改 `target="lead"` 重发成功。
- Live 复现：本会话 `send_message(target="Probe")` → 同样失败；`target="probe"` 正常。
- 根因：`orchestration.rs:704` spawn 时 `name.to_lowercase().replace(' ', "-")` 作为 map key；`enqueue_message`（:858）精确匹配。`list_agents` 同时暴露 `name="Lead"` 与 `target="lead"`，模型易误用 name。
- 建议：lookup 前对 target 做同样归一化（或错误信息提示正确 target）。靠 review + 后续任务实施。

### D2（使用陷阱）drain_inbox 会消费任意成员信箱

- `pa_01a1b66f` 中 Lead 于 14:58:57 `drain_inbox(target=dev-tui)` 把派发给 dev-tui 的任务消息消费掉，被迫 14:59:06 重发。
- 语义符合工具描述（读取并消费），但 Lead 只应 drain 自己的信箱；队友信箱由驱动循环消费。建议在 Lead 提示词中强调。

## 3. Live 实测（当前会话，probe 队友）

| 步骤 | 结果 |
| :--- | :--- |
| team_task_create（添加） | ✅ task-d798f7be，rev=1 pending |
| send_message 派发（分派+唤醒） | ✅ delivered=true, driving |
| probe claim（领取） | ✅ rev 1→2，owner=probe，in_progress |
| probe send_message→lead（回复） | ✅ drain_inbox(lead) 收到「claim 返回 revision=2」 |
| probe complete（更新） | ✅ rev 2→3，completed |
| CAS 守卫（错误 expected_revision=1） | ✅ 拒绝：`CAS revision mismatch: expected 1, got 3` |
| delete 清理 | ✅ rev=4，status=deleted |

测试残留已清理（任务软删除）；probe 队友保留为 inactive 可复用。

## 4. 修复落地（2026-11-08 第二轮）

决策记录：`.agents/notes/implemented/bug-fix/2026-11-08-team-link-target-normalization-and-inbox-scope.md`。

| 项 | 修复 | 落点 |
| :--- | :--- | :--- |
| D1 | `normalize_target`（trim+小写+空格转连字符）成为登记与查找唯一规则 | `orchestration.rs`：register/enqueue/interrupt/drain/teammate_session_id |
| D2 | 模型侧 drain 只允许调用者自己的信箱；`context.session_id` 反查身份（宿主⇒lead），跨成员返回 `inbox_scope_violation` 且不消费 | `tools.rs::execute_drain_inbox` + `orchestration::teammate_target_for_session` |
| D3（新） | `team_task_list` 过滤软删除任务（此前 deleted 任务泄漏进 /tasks 并被投影降级为 Pending） | `orchestration.rs::team_task_list` |

回归：`tests/team_link_fixes_test.rs`（D1/D3）、`tests/team_drain_scope_test.rs`（D2，含真实驱动+拒绝不消费+lead 回执保留）。

## 5. /tasks 与 /agents 面板核查

- **/tasks**：数据源 `team_task_list` 正确透传 owner/blocked_by/write_scopes；唯一缺陷是 D3（已修，live 实证两条 deleted 探针任务曾列板）。
- **/agents 花名册**：数据全部来自真实运行时（list_agents/收件箱/驱动态/会话缓存），lead 固定首位，状态文案与 serde 口径一致 —— **正确**。注意团队状态是**进程内**的：TUI 重启后花名册只剩 lead（设计如此，非缺陷）。
- **teammate 会话详情入口**：机制完整且数据在位 —— W3b 共享会话存储生效（live证据：probe 会话 `subagent-382d5ce8` 与宿主会话同落 `~/.local/share/PonyAgent/sessions.db`，history=2），`session_is_loadable` 判据可通过。进不去的三种真实原因（均有可读写提示）：
  1. **流式守卫**：宿主 turn 在飞时 Enter 被拦（"本轮进行中：先停止或等待结束后再切换会话"）——与 Lead 对话期间必现，需等本轮结束；
  2. **默认选中 lead**：面板打开时选中第 0 项 lead，直接 Enter 提示"Lead 即宿主主会话，无需进入"，须先 ↓ 选中队友；
  3. 队友从未成功运行 ⇒ "尚无独立会话（未成功运行）"。
  若空闲时选中队友仍进不去，请记录状态行提示文案再报（提示会区分"无会话"与"宿主不可加载"）。

## 6. 复验（2026-11-08 第三轮）

- **回归扩面重跑**：10 个测试二进制 **152 passed / 0 failed**（exit 0）——在原 8 个之外新增
  `w3_team_visibility_test`（96 项，含 /tasks 状态映射、字段透传、/agents 名册选择与命令解析）、
  `team_task_order_contract`、`team_history_acceptance`。面板投影契约与修复同批验证。
- **构建产物含修复**：`target/debug/pony`（2026-11-08 16:53:54）检出 `normalize_target` 与
  `inbox_scope_violation` 符号 ⇒ D1/D2/D3 已入构建。
- **运行实例仍是旧二进制（/tasks 面板状态此刻仍不正确）**：现存活 pony 进程启动于 11:41/11:56，
  早于 16:53 构建；live `team_task_list` 仍返回两条 `status=deleted` 探针任务（旧代码行为实证）
  ⇒ /tasks 面板此刻仍把它们投影成 Pending。**重启新二进制后**：两条 deleted 任务被过滤，
  看板走「当前没有任务」空状态；/agents 只剩 lead（团队状态进程内，设计如此）。
- **/agents 花名册 live 数据正确**：Lead=running（本 turn 在飞）、probe=inactive，与真实驱动态
  一致；投影文案与 `TeammateStatus` serde 口径逐字一致（projection.rs:87-96）。