# Session Control Plane 与 Audit Surface 架构基线

## 目的

本文件把当前围绕 trace、session 可控、hooks 可拓展性的讨论结论，进一步沉淀为 `Session Control Plane` 方向的正式架构基线，用于指导后续任务拆分、OpenSpec 约束与实现顺序。

本文件不是对既有 foundation 的替代，而是在下列基线之上补一层更贴近当前开发重点的约束：

- `turn lifecycle` 是执行真相源
- `hooks` 只能挂在稳定 boundary 上
- `trace / checkpoint / history / rollback / UI` 必须消费同一份 canonical contract

参见：

- [Turn Lifecycle、Hooks 与 Recovery 架构基线](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/turn-lifecycle-hooks-and-recovery.md)

## 正式命名

后续统一使用：

- `Session Control Plane`

它指的是用户对当前会话进行的控制动作及其读面，包括：

- stop
- continue
- resume
- replay
- checkout history node
- restore branch head
- fork from history node
- switch branch

不再使用过于宽泛的“session 状态管理”作为主命名，因为这里的核心不是被动状态保存，而是“可控动作 + 审计读面 + 恢复边界”。

## 核心判断

### 1. hooks 的可拓展性风险，根因不在 hooks 数量，而在挂载点是否稳定

工业项目里，`prompt` 负责软控制，`hooks` 负责硬控制。

因此真正的问题不是“未来会不会有更多 hooks”，而是：

- hooks 是挂在 canonical lifecycle / control boundary 上
- 还是挂在临时实现细节上

只有前者才具备长期可拓展性。

约束语句：

> Hooks are controlled extensions on canonical lifecycle and control boundaries.

### 2. trace 与 session control 的不稳定，通常来自缺少同一份 audit read model

当后端、前端、持久化各自维护一套解释逻辑时，就会出现：

- 指标统计口径漂移
- reload 后证据丢失或被二次猜测
- UI 能操作，但不能解释“刚刚到底发生了什么”
- 修复旧 bug 时继续引入新补丁

因此下一阶段的重点，不是继续在 UI 上叠反馈，而是收口一层稳定的：

- `persisted evidence`
- `audit summary`
- `read-plane contract`

### 3. Session Control Plane 必须依赖后端真相源，而不是前端私有推理

前端可以做展示和交互，但不能成为以下语义的仲裁方：

- 当前 control action 命中了哪条 boundary
- 是否发生 degraded restore
- workspace rollback 是否真正 applied
- 当前 visible node / active branch / branch head 是什么
- 最近一次 history-state / run-control evidence 的结论是什么

这些都必须从后端的 canonical read-plane 投影出来。

## Session Control Plane 的分层

### 1. Command Boundary

表示真正被系统承诺的控制动作入口，例如：

- stop requested
- run resume
- history checkout
- branch restore
- branch fork
- branch switch

### 2. Evidence Layer

表示上述 boundary 执行后留下的持久化证据，至少包括：

- boundary
- command kind
- result kind
- blocked / degraded
- duration
- request / resolved target 摘要

### 3. Audit Summary Layer

表示专门给 UI、调试和验收消费的摘要层，而不是把整条 evidence chain 原样压给前端做二次解释。

建议摘要至少回答：

- 最近一次 control action 是什么
- 命中了哪类 boundary
- 成功、阻断还是 degraded
- 当前 session 处于 live / paused / historical / historical_dirty 哪种受控态
- 当前 visible node / branch / workspace rollback 结果是什么

### 4. Frontend Interaction Layer

前端职责是：

- 展示当前可执行动作
- 展示最近一次 control action 的结果
- 展示 disabled reason
- 展示 transcript-only / transcript+workspace 的恢复结果

前端不负责：

- 从散乱 flags 重建 control 结论
- 从缺失 evidence 反推恢复事实
- 自己定义第二套 session 状态机

## 当前完成态

`Session Control Plane` 当前已经完成两张 summary family 落地卡：

- `PA-042`：history-control summary family
- `PA-043`：run-control summary family

对应 canonical specs：

- [session-control-audit-surface-and-history-evidence-summary/spec.md](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/session-control-audit-surface-and-history-evidence-summary/spec.md)
- [run-control-audit-surface-and-summary-first-explainability/spec.md](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md)

当前已成立的实现约束：

- history-control 与 run-control 都已经具备独立 summary family，而不是共享一份模糊 control 文案
- summary 只是 audit read-model，不是新的 truth-source
- `start_graph_run_stream` 只有在 `replay_from_checkpoint / restart_from_checkpoint` 语义下才允许进入 run-control summary
- 普通首轮启动仍属于通用 turn/run lifecycle，不回吞进 run-control summary
- 前端已经完成 summary-first 数据源切换，但没有借机重做 `PA-037` 已冻结的交互结构

## 可拓展性原则

### 1. 先定 boundary，再扩 hook

新增 hook 前必须先回答：

- 它附着在哪个 canonical boundary
- 该 boundary 的输入摘要是什么
- 该 boundary 的输出事实是什么
- 失败后是否允许 blocked / degrade / fail-command
- 需要留下什么 persisted evidence

如果这些问题回答不清，就不应该先加 hook。

### 2. hooks 只能扩策略，不能重写真相源

hooks 可以：

- observe
- guard
- limited transform

hooks 不可以：

- 直接改写 session truth-source
- 自行成为 restore / replay / branch / rollback 的第二仲裁源
- 用 evidence 反向重建缺失的控制结论

### 3. UI 只消费 audit surface，不直接消费内部执行细节

对前端开放的应该是：

- canonical state
- control summary
- evidence summary

而不是：

- 任意内部 flags
- 未承诺的执行中间态
- 需要 UI 自行拼装的多来源局部字段

## 已完成范围

虽然 `Session Control Plane` 的总语义同时覆盖 run-control 与 history-control，但这轮已完成范围是分两张卡收口的：

### `PA-042 v1`

- 建立统一的 `history-state audit summary contract`
- 把 summary 投影到 `session snapshot / runtime view / history-control responses`
- 把前端 history explainability 主展示改成 `summary-first consumption`

### `PA-043 v1`

- 建立统一的 `run-control audit summary contract`
- 把 summary 投影到 `session snapshot / runtime view / run-control responses`
- 为 replay/restart start 补 `start_reason` 区分，并明确排除普通首轮 `start_graph_run_stream`
- 把前端 run-control explainability 主展示改成 `summary-first consumption`

两张卡共同明确不做：

- 重写 `PA-037` 已成立的按钮编排、disabled reason、状态语言
- 新增第二套 restore / replay / checkpoint / phase 仲裁源
- 让 summary 反向成为 truth-source

## Summary 分层

为避免 summary 反向污染 truth-source，后续实现必须拆成两层：

### 1. Action Evidence Summary

表示“最近一次 control action 留下的持久化审计摘要”，字段必须绑定到底层 evidence，不允许随当前 session 现态漂移。

建议最小必填字段：

- `status`
- `source_family`
- `command_kind`
- `boundary`
- `result_kind`
- `summary`
- `elapsed_ms`
- `blocked`
- `degraded`
- `evidence_id`
- `observed_at_ms`

可选字段：

- `requested_node_id`
- `requested_branch_id`
- `resolved_node_id`
- `resolved_branch_id`
- `workspace_rollback_capable`
- `workspace_rollback_applied`
- `degradation_reason`

### 2. Current Context Projection

表示“读取 summary 时当前 session 的受控上下文”，只用于辅助解释，不得伪装成 action 自身的持久化结果。

建议字段：

- `history_cursor_mode`
- `visible_node_id`
- `active_branch_id`
- `branch_head_node_id`

额外约束：

- 这部分必须在命名或结构上与 action summary 分离
- 前端不得把 current context 当作历史动作结果回写

## 已完成能力

### 已完成 foundation

当前仓库已经具备：

- turn lifecycle contract
- trace persistence / recovery contract
- hooks foundation 与 stable-boundary dispatch
- checkpoint lifecycle boundary
- session control UX 第一轮闭环
- run / memory / planner / capability / history-state hooks
- history-control summary-first explainability
- run-control summary-first explainability

### 当前 `Session Control Plane` 已具备的工程能力

当前已经可以稳定回答：

- 最近一次 history-control 动作是什么，命中了哪条 boundary，结果为何 blocked / degraded
- 最近一次 run-control 动作是什么，命中了哪条 boundary，结果为何 blocked / degraded / replay_required
- reload 后，如果 evidence 仍在，最近一次 control summary 是否还能读回
- 如果 evidence 缺失，系统是否会明确表现为 missing/unavailable，而不是伪造成功结论
- 当前上下文与历史动作证据是否已明确分层，而不是互相冒充

## 下一阶段建议

### 当前最合理的下一步

下一阶段更适合聚焦：

- `Session Control Plane` 的 monitor / drilldown 下钻读面
- 更细的 run-control / history-control 审计可视化
- 挂在既有 stable boundary 上的更强 guard / policy / enterprise controls

即：

- 不回灌已完成的 `PA-042 / PA-043`
- 继续沿用 `summary read-model + truth-source guardrail + stable hooks boundary` 这套架构原则

## 验收原则

后续相关任务的验收至少要证明：

1. summary 来自后端真相源，而不是前端拼装
2. `action evidence` 与 `current context` 分层明确，不互相伪装
3. reload 后 action summary 与 underlying evidence 保持一致
4. 缺失 evidence 时系统只表现为“证据缺失”，而不是伪造恢复结论
5. hooks 扩展没有破坏既有 cursor / rollback / branch / checkpoint 真值
6. 前端能清楚回答“刚刚执行了什么控制动作，结果如何，为什么”

## 与现有任务关系

- `PA-031 ~ PA-036`：turn lifecycle、trace、checkpoint、terminal truth-source 基线
- `PA-037`：session control UX 第一轮显式化
- `PA-038 ~ PA-041`：hooks 扩展到 run / memory / planner / capability / history-state
- `PA-042`：history-control audit surface 与 summary-first explainability
- `PA-043`：run-control audit surface 与 summary-first explainability
- 下一轮如继续扩展，应新增独立任务承接 monitor / drilldown 或更细控制策略，而不是回灌已关闭任务
