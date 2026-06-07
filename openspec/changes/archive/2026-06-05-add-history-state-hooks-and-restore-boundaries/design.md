# Design: History-State Hooks And Restore Boundaries

## 背景

当前 hooks 主线已经完成：

- turn foundation：`PA-033 / PA-035`
- run / execution-control：`PA-038`
- memory write / persisted side effect：`PA-039`
- planner / capability mediation：`PA-040`

但 `history checkout / restore / fork / switch branch` 仍然完全停留在 session/control-plane 的控制命令语义里，缺少 hooks contract。与此同时，这条线已经具备：

- 稳定的 `SessionStore` 真边界
- 明确的 `HistoryCheckoutMode / HistoryCheckoutStatus`
- 统一的 `HistoryCheckoutResponse / RestoreBranchHeadResponse / ForkFromHistoryNodeResponse / SwitchHistoryBranchResponse`
- file-backed persistence 与前端 degrade feedback

因此它已经足够成为独立的 post-foundation hooks change。

## 设计目标

1. 只把 hooks 挂在稳定的 history-state control boundary 上
2. 不让 hooks 直接改 history graph、cursor 或 workspace rollback 真值
3. 为 history-state 切换建立最小 persisted audit chain
4. 保持 degrade / rollback 合同仍以既有 truth-source 为准

## 非目标

- 不在本卡引入新的 history scheduler 或 workflow
- 不在本卡重写 `PA-028 / PA-032 / PA-037` 已收口的 history graph / degrade / UX 逻辑
- 不在本卡把 workspace rollback 实现为 hooks 自带副作用执行器

## 稳定边界

本卡只覆盖以下四类 canonical boundary：

- `history_checkout`
- `branch_restore`
- `branch_fork`
- `branch_switch`

建议的第一版 hook point：

- `HistoryCheckoutStart`
- `HistoryCheckoutResolved`
- `BranchRestoreStart`
- `BranchRestoreResolved`
- `BranchForkStart`
- `BranchForkResolved`
- `BranchSwitchStart`
- `BranchSwitchResolved`

其中：

- `Start` 用于 guard / observe / limited transform
- `Resolved` 用于 observe persisted evidence
- 若某个点无法绑定到稳定命令结果，则优先收紧合同，不新增 hook 点

## Normalized Envelope

history-state hooks 只读取 normalized history-control envelope。第一版建议字段：

- `command_kind`
- `session_id`
- `requested_node_id`
- `requested_branch_id`
- `requested_checkout_mode`
- `resolved_node_id`
- `resolved_branch_id`
- `transcript_restore_applied`
- `workspace_rollback_capable`
- `workspace_rollback_applied`
- `degraded`
- `degradation_reason`
- `history_cursor_summary`

约束：

- `workspace_rollback_capable / workspace_rollback_applied / degradation_reason` 为只读 truth-source 字段
- patch 若存在，第一版只允许改请求侧字段，例如受控地收紧 `requested_checkout_mode`
- hooks 不允许直接 patch history graph、history nodes、history branches、cursor persisted state

## Evidence And Persistence

history-state hooks 不能只停留在命令响应对象里。第一版最小要求：

- history-state evidence 必须进入 session truth-source 可 reload 载体
- control-plane / runtime view 至少能读回最近一次 history-state control evidence
- 这批 evidence 只作为 persisted audit chain，不参与 restore、submission 或 history cursor 的仲裁真值
- persisted evidence 至少包含：
  - boundary
  - result kind
  - duration
  - blocked / degraded 摘要
  - request/response 标识摘要

推荐第一版载体：

- 在 session snapshot 增加独立的 history-state control evidence 集合
- runtime view 仅投影最近一条或当前相关 evidence 摘要，避免把内部历史过度压给前端

## Failure Policy

history-state hooks 继续遵守受控扩展原则：

- guard 可阻断 history-state 命令
- observe 不得改变结果
- transform 只允许改白名单请求字段
- fail-turn / fail-command 策略必须显式声明

额外约束：

- hooks 不得把 degrade path 伪装成 workspace rollback 成功
- hooks 不得绕开既有 `HistoryCheckoutStatus` 或 `HistoryCursor` 真值
- 缺少 persistence evidence 时，reload 只能退回“无 hooks evidence”，不得伪造恢复结论，也不得据此重建 restore 决策

## Read-Plane

第一版至少保证：

- control-plane response 可带当前 history-state control evidence
- session snapshot / file roundtrip 可读回
- runtime view 可读回最近一条 evidence
- 前端若未消费详细 evidence，不阻断本卡关闭；但 read-plane 必须后端可验证

## 验证策略

最小验证矩阵：

- `checkout_history_node` 命中 hook boundary
- `restore_branch_head` 命中 hook boundary
- `fork_from_history_node` 命中 hook boundary
- `switch_history_branch` 命中 hook boundary
- guard 阻断 checkout/restore 时，不生成新的 resolved evidence、不改 history cursor 真值
- transcript+workspace 请求在 rollback 不支持时，hooks evidence 与 degrade 结果一致
- 缺少 hooks persistence evidence 时，reload 后只能表现为“无 hooks evidence”，不能重建 restore 结论
- file-backed reload 后能读回 history-state evidence
- control-plane 与 runtime view 对同一条 history-state evidence 的投影保持一致

## 与现有任务边界

- `PA-028`：history graph / branch / cursor 真边界
- `PA-032`：recovery / degrade contract
- `PA-037`：session control UX 与反馈
- `PA-033 / PA-035`：hooks foundation 与 turn stable-boundary dispatch
- `PA-041`：只负责把 history-state control boundary 正式接入 hooks
