# Design: Run Control Audit Surface And Summary-First Explainability

## 背景

当前项目已经具备以下基础：

- `PA-037`：session control 第一轮显式交互与统一状态语言
- `PA-038`：`submission_plan / wait_user / stop_requested / run_resume` 的 persisted boundary evidence
- `PA-042`：history-control 的 audit summary 分层、truth-source guardrail 与前端 summary-first read model

也就是说，系统并不缺 run-control boundary 和 evidence，而是缺一层更适合 UI、调试和验收直接消费的稳定摘要层。

## 设计目标

1. 为 run-control 建立统一 audit summary 读面
2. 让 summary 明确来源于 persisted truth-source
3. 让 runtime view、session snapshot、run-control response 共享同一 summary contract
4. 减少前端对 submission plan / checkpoint / boundary evidence 的私有推理
5. 保持 summary 是只读审计面，而不是新的 run arbitration source

## 非目标

- 不新增新的 run-control command 或 checkpoint command
- 不替换底层 `control_boundary_evidence`、`submission_plan`、`execution_checkpoint` 原始载体
- 不在本卡重做 `PA-037` 已经成立的显式按钮、disabled reason 和状态语言
- 不在 `PA-043 v1` 中重做 session control 的视觉设计或信息架构
- 不让前端直接展示完整 run evidence chain 作为主交互

## 命名

后续统一使用：

- `RunControlAuditSummary`
- `Run Control audit surface`

其中：

- `Run Control audit surface` 表示用户对当前 graph run / checkpoint 的控制动作及其反馈面
- `RunControlAuditSummary` 在 `PA-043 v1` 中只表示后端投影给前端和调试读面的最近一次 run-control 动作摘要

## v1 Scope

`PA-043 v1` 只覆盖以下 command kind：

- `stop_graph_run`
- `continue_graph_run_stream`
- `resume_graph_run_stream`
- `start_graph_run_stream`（仅限 replay / restart 语义）

其中 `start_graph_run_stream` 只在 run-control 语义下表示 `replay / restart from checkpoint decision`，不回吞普通首轮启动的通用 turn 行为。

为避免该语义只停留在文案说明，`PA-043 v1` 必须为 `start_graph_run_stream` 补充可判定的 run-control start 语义字段。建议固定使用：

- `start_reason`

其值域至少区分：

- `initial_turn`
- `replay_from_checkpoint`
- `restart_from_checkpoint`

约束：

- 只有当 `start_reason` 为 `replay_from_checkpoint` 或 `restart_from_checkpoint` 时，本次 `start_graph_run_stream` 才允许进入 `RunControlAuditSummary`
- 普通首轮 `start_graph_run_stream` 必须保持在通用 turn/run 启动语义中，`PA-043 v1` 不为其生成 run-control summary

`PA-043 v1` 明确不覆盖：

- history checkout / restore / fork / switch
- 新的 session scheduler / workflow mode
- 新的 monitor 可视化重做

## Summary Contract

为避免 contract 模糊，`PA-043 v1` 应把 summary 明确拆成固定顶层结构：

- `action_evidence_summary`
- `current_context_projection`

两层必须同时存在；当 evidence 缺失时，也必须返回固定顶层 shape，而不是回退成松散字段集合。

### 1. Action Evidence Summary

表示最近一次 run-control 动作的持久化审计摘要。该层必须与 evidence 绑定，reload 后不得随当前 session 现态漂移。

required:

- `status`
- `source_family`
- `command_kind`
- `boundary`
- `result_kind`
- `summary`
- `target_summary`
- `elapsed_ms`
- `blocked`
- `degraded`
- `evidence_id`
- `observed_at_ms`

optional:

- `run_id`
- `turn_id`
- `checkpoint_turn_id`
- `checkpoint_kind`
- `recovery_mode`
- `projected_command`
- `request_summary`
- `degradation_reason`
- `start_reason`

其中：

- `status` 值域在 v1 固定为：`available | missing`
- `source_family` 在 v1 固定为 `run_control`
- `command_kind` 值域在 v1 固定为：`stop_graph_run | continue_graph_run_stream | resume_graph_run_stream | start_graph_run_stream`
- `boundary` 值域必须来自既有 run-control stable boundary vocabulary，不允许前端自定义新字面量
- `result_kind` 值域必须来自既有 run-control resolved result vocabulary，不允许各读面重命名
- required 字段在 `SessionSnapshot / SessionRuntimeView / run-control responses` 中必须逐字一致
- `target_summary` 为 required，确保前端不需要回落到兼容字段链补目标解释
- `request_summary` 在 `blocked=true` 时必须存在，用于解释“尝试执行了什么请求”
- `recovery_mode` 在 `command_kind = start_graph_run_stream` 且 `start_reason` 为 `replay_from_checkpoint | restart_from_checkpoint` 时必须存在
- `start_reason` 在 `command_kind = start_graph_run_stream` 时必须存在
- 当 `status = missing` 时，required 字段仍必须保留固定 shape；其中：
  - `summary` 应返回 `summary unavailable` 或等价缺失语义
  - `target_summary` 应返回 `target unavailable` 或等价缺失语义
  - `blocked` 与 `degraded` 默认读取为 `false`
  - `evidence_id` 应返回稳定的空值语义，例如 `null`

### 2. Current Context Projection

表示读取 summary 时当前 session 的只读上下文投影，不属于动作证据本身。

required:

- `phase`
- `checkpoint_status`

optional:

- `active_run_id`
- `checkpoint_kind`
- `checkpoint_recovery_mode`
- `submission_plan_command`

其中：

- 该层字段不得写入 `action evidence summary`
- `submission plan / checkpoint / graph phase` 相关字段只允许读取既有 truth-source，不允许由 summary 自己重算
- 前端必须把“动作证据”和“当前上下文”分开展示或分开解释
- `current_context_projection` 是 read-time projection，不属于 file-backed persisted evidence 的 roundtrip 比较对象

## 生成规则

第一版 summary 采用“由后端从既有 persisted evidence 与 run truth-source 统一投影”的策略：

1. 优先读取最近一次可解释的 run-control evidence
2. 结合当前 session checkpoint / submission plan / graph phase 真值补齐只读 current context
3. 若 evidence 缺失，只返回“summary unavailable / evidence missing”级别状态
4. 不允许根据缺失 evidence 反向重建 continue / resume / replay 结论

## 投影位置

第一版至少接入：

- `SessionSnapshot`
- `SessionRuntimeView`
- `GraphRunControlResponse`
- `GraphRunStreamStartResponse`

其中：

- `GraphRunStreamStartResponse` 只有在 `start_reason = replay_from_checkpoint | restart_from_checkpoint` 时才暴露 `RunControlAuditSummary`
- 普通首轮启动响应不得因为复用同一 response shape 而伪装成 run-control summary source

## 前端消费约束

前端主展示应优先消费 `RunControlAuditSummary`，而不是：

- 直接遍历 `control_boundary_evidence`
- 先看旧兼容字段，再私有拼合 continue/resume/replay 结果
- 从缺失 summary 倒推出 checkpoint arbitration 事实

兼容字段可以保留给测试桩或过渡逻辑，但主 UI 读面必须切到 summary 优先。`PA-043 v1` 只要求完成数据源切换，不重做 `PA-037` 已成立的按钮、disabled reason 和状态语言。

前端范围进一步固定为：

- 不新增 section / card / panel
- 不调整 `HomeSessionSidebar` 的结构、命名或分组
- 不改 `HomeWorkspace` 的 CTA 编排、标签或 disabled reason
- 只允许在既有 status card / feedback strip 内切换为 summary-first explainability
- 若需要区分 `action_evidence_summary` 与 `current_context_projection`，只能在既有展示位中完成，不得借机扩张信息架构

## 失败与缺失语义

### blocked

- 若最近一次 run-control action 被 hook guard 或其他稳定策略阻断，summary 必须显式标明 `blocked=true`
- 同时保留请求摘要，便于 UI 解释“试图做什么，但未成功”

### degraded

- degraded 继续以既有 submission plan / checkpoint truth-source 为准
- summary 只能投影该结论，不能伪造 resume 已成功或 replay 已完成

### evidence missing

- 若 reload 后缺失底层 evidence，summary 只能表现为 unavailable/missing
- 不得反向构造“上次应该是 resume 成功”之类推断
- 若同时存在 blocked 请求但 evidence 缺失，也只能表现为缺失，而不是拼接出伪 blocked 结论

## Truth-Source Guardrail

必须显式验证：

- 篡改或缺失 summary 不会改变 submission plan 真值
- 篡改或缺失 summary 不会改变 checkpoint 真值
- 篡改或缺失 summary 不会改变 graph run phase 真值

summary 不是仲裁输入，只是投影结果。

## 持久化与 Roundtrip 边界

`PA-043 v1` 必须明确区分“持久化的动作证据”与“读取时的当前上下文投影”：

- file-backed persistence 与 reload roundtrip 必须稳定保真的对象是 `action_evidence_summary`
- `current_context_projection` 必须在读取时从既有 truth-source 重新投影
- reload 验收中的“一致性”默认只比较 `action_evidence_summary` 的 required fields
- 若 reload 后 session phase / checkpoint / submission_plan_command 已变化，允许 `current_context_projection` 变化，但不得改写历史 action evidence
- 任何实现都不得把完整 summary 对象整体冻存为新的 truth snapshot

## 验证策略

最小验证矩阵：

- stop summary
- continue / resume / replay(start) summary
- ordinary initial start SHALL NOT produce run-control summary
- blocked summary
- degraded / replay_required summary
- blocked + evidence missing 组合负向路径
- degraded + replay_required 组合路径
- file-backed reload 后 summary roundtrip
- reload 后 current context 漂移但 action evidence 保持不变
- runtime view 与 command response 投影一致
- evidence missing 负向路径
- truth-source non-regression
- 前端能稳定展示最近一次 run-control action、boundary、blocked/degraded 结果和当前上下文摘要

## 与现有任务边界

- `PA-037`：保留既有显式 CTA 与基本状态语言
- `PA-038`：提供 run-control evidence 基线
- `PA-042`：已完成 history-control summary read-model
- `PA-043`：只负责把 run-control evidence 收口成统一 audit summary 读面与 explainability surface
