# Agent Note: 审批类控制请求可恢复：宿主面板 + 批准后授权并续轮（wave-65）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0077-approval-resume-host-panel-and-continuation.md` 之归档快照，仅供溯源。

## 背景

`ApprovalRequired`（路径越界读取、外部工具声明需审批、用户规则 Ask）在治理 dispatcher 上会持久化一个
`PendingControlRequest(kind=Approval)` 并把工具结果置为 `control_outcome_pending`。运行时曾按
`approval_resume_unavailable` 直接把整轮判死（`runtime/turn_control.rs` 的 W1b 收口，2026-11-05
提交 `a213fd7d`），理由是当时没有「批准后重放该工具调用」的实现。真实后果（会话
`pa_01a1b122-b0de-7162-8c23-e29e27409417`）：一次工作区外 `Read` 让 40.2s、13 跳工具调用的整轮
作废，会话只剩一条失败摘要，任务无法继续。同时 TUI 的审批面板与 `authorize_path` 早已存在，却因
`list_pending_asks` 只读 Ask 持久化 store 而永远拿不到 Approval，形成死代码。

## 候选方案

- **A 运行时真重放**：批准后由 runtime 用同一 `call_id` 重新执行该工具调用。优点：语义最完整；缺点：
  dispatcher 只持久化 `final_args_digest` 而非原始实参，重放需要新增实参持久化 + 重放 API + 与
  terminal-once / handoff 记账对齐，改动面覆盖 runtime/dispatcher/graph，且与 Ask 的注入机制不同构，
  回归风险显著高于收益。落选。
- **B 宿主面板 + 批准后授权 + 续轮（采纳）**：运行时只把 Approval 挂起变成**可恢复挂起**
  （`turn:suspended`）；宿主列出待决审批，批准时写入 read 作用域路径授权并消费请求，然后以上一轮
  相同会话发起一次**宿主续轮**（固定文案），模型在已授权的路径上重试工具调用。优点：复用 Ask
  恢复已落地的「真实 graph run + 宿主续轮」基建，无需新增实参持久化与重放 API；权限判定本身一
  行不改。缺点：严格说不是「同一调用重放」，而是「批准后允许重试」，需要一份宿主固定文案。
- **C 降级为工具级拒绝**：把 Approval 变成 provider 可消费的 Deny，让模型改道。优点：最小；缺点：
  用户永远没有审批入口，产品语义退化为「工作区外一律不可读」，与 PA-080「未授权触发宿主审批」
  的既定设计相悖。落选（仅作为未接线宿主的保底行为保留）。
- **D 维持 fail-fast**：改动为零，但会话仍会被一次越界读取整轮打死。落选。

## 决策

采纳 B，并以**能力位**（`AgentRuntimeBuilder::host_approval_resume`，默认 `false`）限定生效面：
只有显式接线的宿主（当前仅 TUI）把 **路径授权类** `Approval` 挂起判为可恢复；桌面（`src-tauri`）与
CLI 行为逐字不变，仍 fail-fast。非路径审批（用户规则 Ask、外部工具声明需审批、写执行保守抬升）**不**
进入可恢复挂起——批准被消费后不产生豁免，续轮重试必再审批，放开只会得到「批准→续轮→再审批」循环；
其恢复需要 dispatcher 级一次性豁免，登记为后续工单（`.dev-team/backlog-approval-resume-followups.md` F-1）。
审批请求带结构化 `options`（`approvalKind`/`tool`/`scope`；**仅路径授权类**额外带 `path`，非路径审批不带 path 键），宿主经 `list_pending_approvals`
取出（**过滤已过期请求**，否则面板会在 CAS 必然失败后每 500ms 重弹）、经 `approve_control_request`
（批准，CAS→Consumed）或 `cancel_approval_request`（拒绝，CAS→Cancelled，带 kind 闸门）收口；
批准侧只写 read 作用域路径授权。

## 影响

- 新增宿主接口 `list_pending_approvals` / `cancel_approval_request`（无 nonce、会话作用域）与 TUI
  事件 `ApprovalResumeTurn`；`PendingControlRequest` 结构不变（上下文走既有 `options` 字段）。
- 桌面端后续若要接审批面板，必须显式打开能力位并实现面板，否则维持 fail-fast——这避免了「挂起后
  无人恢复」的死等。
- 批准后的续轮是**新的一轮**（宿主固定文案），不是同一调用的重放；该语义差异写进契约矩阵
  `.dev-team/contract-matrix-approval-resume.md`，后续若实现真重放应以新 ADR 接替本篇。

