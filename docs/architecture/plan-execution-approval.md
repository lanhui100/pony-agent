# Plan Mode 审批（会话级协作模式，2026-11-27 重构）

状态：已实施（ADR `2026-11-27-plan-mode-dsh-alignment-and-task-count`，1A + 2A 方案）。
本文取代原《Plan Execution Approval（W36 / PA-141）》——那套 `Draft → Executing` 的步骤机审批面已随
`PlanStore` 一并下线。

## 1. 问题（为什么推翻旧审批面）

旧实现把 Plan 做成了带 `steps[]` / `complete_step` / `revision` CAS 的**第二套任务清单**，并给它的
`Draft` 态加了写锁。由此产生两个结构性问题：

1. **语义串名**：`TodoWrite` 的进度被 `context.rs` 回灌成 `Plan: x/y completed`，模型同时看到
   「Plan 工具（步骤机）」与「Plan 回灌（其实是 Todo）」，长期在两者之间犯错。
2. **审批面是为步骤机而生**：`plan_execution_approval` 绑定 `(session_id, plan_id)` + TTL + 一次性消费，
   其唯一作用是允许 `Draft → Executing`。一旦 Plan 不再是步骤机，这套证据就没有承载对象。

## 2. 裁决：Plan 是会话级协作模式

对齐 DeepSeek Harness `packages/plan/plan-mode`：Plan 只有 `active: boolean`、一份 Markdown 方案正文与
待生效的 `pending` 意图，**不承载任何步骤进度**。

| 概念 | 工具 / 入口 | 唯一职责 | 含步骤清单 |
|---|---|---|---|
| **Plan（计划模式）** | `/plan`、`/plan off`、`ExitPlanMode` | 会话级协作模式：先出方案，审批通过才解锁写/执行 | **否** |
| **Todo（个人步骤）** | `TodoWrite` / `TodoList` | 单智能体会话内步骤拆解与打勾 | 是（无写锁） |
| **Team Task（团队任务）** | `team_task_create` / `update` / `list` | 多智能体看板分工 | 是（团队专用） |
| **Goal（持久目标）** | `create_goal` / `get_goal` / `update_goal`、`/goal` | 跨多轮自动续行的顶层验收目标 | **否** |

## 3. 状态机

```
            /plan（宿主）
  inactive ───────────────▶ pending=true ──(pre-step commit)──▶ active
      ▲                                                          │
      │  /plan off（宿主，立即）                                  │ ExitPlanMode 提交方案
      └──────────────────────────────────────────────────────────┤ （awaitingReview=true）
      │                                                          ▼
      │                                          用户「批准执行」→ resolve_review(true)
      └───────────────────────────────────────────────────────────┘
                              用户「继续规划」→ 保持 active，planApproved=false
```

- **持久化**：进程级 `Mutex<BTreeMap<session, PlanModeState>>` + JSON sidecar
  （`PONY_AGENT_PLAN_MODE_STORE_PATH` 可覆盖；`*.tmp` + rename 原子写），重启后可重建。
- **生效时机**：`request_toggle` 只写 `pending`，`commit_pending` 在下一轮 pre-step 落地
  （DSH「用户选择待提交」语义）；`set_active(false)` 与审核批准则立即生效。
- **一次性**：重新进入模式会清掉上一份方案的 `planApproved`——批准不是长期通行证。

## 4. 硬禁写门禁

`governed_executor::decide_with_context` 第 2 步：

- 判定面 = **权限事实**（`kind ∈ {Write, Execute}` 或 descriptor 声明的
  `WorkspaceWrite` / `WorkspaceExecute` scope），不看工具名——否则外部工具（MCP/插件，
  `kind = External`）只要声明 `workspace_write` 就能在 Plan 模式下绕过写保护。
- 会话键 = 运行期注入的 `DispatchContext.session_id`（模型实参不是会话键）；未绑定会话时按
  「未激活」处理，交由只读闸门兜底，不回落默认/环境会话名。
- 拒绝来源可追溯：`decision_source = "plan_mode_gate"`，`error.code = "permission_denied"`。
- **自豁免**：`ExitPlanMode`（primitive `plan_control`）自身不受本门禁约束，否则模式永远解不开。

## 5. ExitPlanMode（唯一退出审查入口）

- 模型可见名 `ExitPlanMode`（原 `Plan` 槽位），schema 收敛为单 `plan`（Markdown）入参。
- 校验：必须在 Plan 模式下调用（否则 `plan_mode_inactive`）；方案必须以 `# ` 一级标题开头
  （否则 `invalid_plan`）；会话必须来自 dispatch context（否则 `missing_session`）。
- 行为：登记方案并返回 `{awaitingReview: true, approved: false, options: [批准执行, 继续规划]}`；
  **不在工具内判定批准**——批准只能由宿主 `plan_mode_review(true)` 产生。
- 审查交互复用既有 Ask（`WaitingHost`）控制请求通道，因此不需要新工具槽位，内建工具数保持 46。

## 6. 宿主控制面

| 动作 | Rust（`HostControlPlane`） | Tauri IPC | 前端 |
|---|---|---|---|
| 读取状态 | `plan_mode_get(session)` | `plan_mode_get` | `planStore.fetchMode` |
| 进入 / 退出 | `plan_mode_set(session, active)` | `plan_mode_set` | `planStore.setMode`、`/plan`、`/plan off` |
| 落地意图 | `plan_mode_commit(session)` | `plan_mode_commit` | `planStore.commitMode` |
| 审核决定 | `plan_mode_review(session, approved)` | `plan_mode_review` | `planStore.reviewPlan`、`/approve` |

错误码（结构化、fail-closed）：`plan_scope_required`（空会话）、`plan_review_no_plan`
（没有提交方案就批准）。

## 7. 机械收口（测试层，非遥测层）

| 断言面 | 文件 |
|---|---|
| 模式状态机 / 持久化 | `crates/pony-tools/src/plan_mode.rs`（5 例） |
| ExitPlanMode 校验与回执 | `crates/pony-tools/src/exit_plan_mode.rs`（4 例） |
| 写/执行拦截、跨会话、读放行、退出解锁、自豁免 | `crates/pony-agent-core/tests/tools_stop_suite/wave36_draft_gate_session.rs`（9 例） |
| 审批只能宿主产生 / 一次性 / 会话隔离 / 无方案 fail-closed | `crates/pony-agent-core/tests/tools_stop_suite/wave36_plan_execution_approval.rs`（7 例） |
| 控制面与门禁共享同一状态 | `crates/pony-agent-core/tests/runtime_suite/wave23_governance_and_plan_sync_contract.rs` |
| 门禁口径（读 descriptor 权限事实） | `crates/pony-agent-core/tests/tools_stop_suite/wave30b_policy_rules_and_plan_gate_test.rs` |
| 前端模式 store / 面板 / 斜杠命令 | `tests/plan-store.spec.ts`、`tests/AskPlanPanel.spec.ts`、`tests/slash-commands.spec.ts` |

## 8. 相关文档

- 提示词与上下文回灌边界：`docs/architecture/plan-goal-contract.md` §4.1
- 工具面单一真值：`docs/architecture/tool-runtime-descriptor-registry.md`
- 决策记录：`.agents/notes/implemented/architecture/2026-11-27-plan-mode-dsh-alignment-and-task-count.md`
