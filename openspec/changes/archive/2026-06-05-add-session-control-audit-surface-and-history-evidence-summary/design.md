# Design: Session Control Audit Surface And History Evidence Summary

## 背景

当前项目已经具备以下基础：

- `PA-031 ~ PA-036`：turn lifecycle、trace persistence、checkpoint boundary 与 terminal truth-source
- `PA-037`：session control 第一轮显式交互与 degrade feedback
- `PA-041`：history checkout / restore / fork / switch 的 history-state evidence

也就是说，系统并不缺底层 boundary 和 evidence，而是缺一层更适合 UI、调试和验收直接消费的稳定摘要层。

## 设计目标

1. 为 history-control 建立统一 audit summary 读面
2. 让 summary 明确来源于 persisted truth-source
3. 让 runtime view、session snapshot、history-control response 共享同一 summary contract
4. 减少前端对兼容字段和多来源 flags 的私有推理
5. 保持 summary 是只读审计面，而不是新的 control arbitration source

## 非目标

- 不新增新的 history command 或 replay backend command
- 不替换底层 `control_boundary_evidence` 或 `history_state_evidence` 原始载体
- 不让前端直接展示完整 audit chain 作为主交互
- 不在本卡重做 `PA-037` 已经成立的显式按钮和基本状态语言
- 不在 `PA-042 v1` 中新增 run-control summary family 或 stop/resume/continue/replay 的新 response 投影

## 命名

后续统一使用：

- `Session Control Plane`
- `SessionControlAuditSummary`

其中：

- `Session Control Plane` 表示用户可执行的会话控制动作及其反馈面
- `SessionControlAuditSummary` 在 `PA-042 v1` 中只表示后端投影给前端和调试读面的最近一次 history-control 动作摘要

## v1 Scope

`PA-042 v1` 只覆盖以下 command kind：

- `checkout_history_node`
- `restore_branch_head`
- `fork_from_history_node`
- `switch_history_branch`

`PA-042 v1` 明确不覆盖：

- `stop_turn`
- `resume_graph_run_stream`
- `continue_graph_run_stream`
- `start_graph_run_stream`

这些 run-control 读面后续如需统一 summary，应单独立卡，不回灌本卡。

## Summary Contract

为避免 contract 模糊，`PA-042 v1` 应把 summary 明确拆成两层：

### 1. Action Evidence Summary

表示最近一次 history-control 动作的持久化审计摘要。该层必须与 evidence 绑定，reload 后不得随当前 session 现态漂移。

required:

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

optional:

- `command_kind`
- `requested_node_id`
- `requested_branch_id`
- `resolved_node_id`
- `resolved_branch_id`
- `workspace_rollback_capable`
- `workspace_rollback_applied`
- `degradation_reason`

其中：

- `status` 至少区分 `available` 与 `missing`
- `source_family` 在 v1 固定为 `history_state`
- required 字段在 `SessionSnapshot / SessionRuntimeView / history-control responses` 中必须逐字一致
- optional 字段允许因为证据缺失而为空，但不得改变 required 字段语义

### 2. Current Context Projection

表示读取 summary 时当前 session 的只读上下文投影，不属于动作证据本身。

建议字段：

- `mode`
- `visible_node_id`
- `active_branch_id`
- `branch_head_node_id`

其中：

- 该层字段不得写入 `action evidence summary`
- cursor / branch 相关字段只允许读取既有 truth-source，不允许由 summary 自己重算
- 前端必须把“动作证据”和“当前上下文”分开展示或分开解释

## 生成规则

第一版 summary 采用“由后端从既有 persisted evidence 与 session truth-source 统一投影”的策略：

1. 优先读取最近一次可解释的 control evidence
2. 结合当前 session cursor / branch 真值补齐只读 current context
3. 若 evidence 缺失，只返回“summary unavailable / evidence missing”级别状态
4. 不允许根据缺失 evidence 反向重建 restore 结论

换句话说：

- action summary 是 read model
- evidence 是 audit chain
- current context 是现态投影
- cursor / rollback / branch 是 truth-source

三者职责不能混淆。

## 投影位置

第一版至少接入：

- `SessionSnapshot`
- `SessionRuntimeView`
- `HistoryCheckoutResponse`
- `RestoreBranchHeadResponse`
- `ForkFromHistoryNodeResponse`
- `SwitchHistoryBranchResponse`

run-level response 不在 `PA-042 v1` 范围内。

## 前端消费约束

前端主展示应优先消费 `SessionControlAuditSummary`，而不是：

- 直接遍历完整 evidence chain
- 先看旧兼容字段，再私有拼合结果
- 从缺失 summary 倒推出 restore/branch 事实

兼容字段可以保留给测试桩或过渡逻辑，但主 UI 读面必须切到 summary 优先。`PA-042 v1` 只要求完成数据源切换，不重做 `PA-037` 已成立的按钮、disabled reason 和状态语言。

## 失败与缺失语义

### blocked

- 若最近一次 control action 被 hook guard 或其他稳定策略阻断，summary 必须显式标明 `blocked=true`
- 同时保留请求摘要，便于 UI 解释“试图做什么，但未成功”

### degraded

- degraded 继续以既有 restore truth-source 为准
- summary 只能投影该结论，不能伪造 transcript+workspace 成功

### evidence missing

- 若 reload 后缺失底层 evidence，summary 只能表现为 unavailable/missing
- 不得反向构造“上次应该是 restore 成功”之类推断
- 测试必须单列该负向场景

## Truth-Source Guardrail

必须显式验证：

- 篡改或缺失 summary 不会改变 cursor 真值
- 篡改或缺失 summary 不会改变 branch 真值
- 篡改或缺失 summary 不会改变 rollback 真值

summary 不是仲裁输入，只是投影结果。

## 验证策略

最小验证矩阵：

- history checkout 成功 summary
- branch restore / fork / switch 成功 summary
- blocked summary
- degraded summary
- file-backed reload 后 summary roundtrip
- runtime view 与 command response 投影一致
- evidence missing 负向路径
- truth-source non-regression
- 前端能稳定展示最近一次 control action、boundary、degraded 结果和当前上下文摘要

## 与现有任务边界

- `PA-037`：保留既有显式 CTA 与基本状态语言
- `PA-041`：提供 history-state evidence 基线
- `PA-042`：只负责把 history-state evidence 收口成统一 audit summary 读面与 explainability surface
