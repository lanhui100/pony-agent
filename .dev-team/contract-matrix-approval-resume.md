# wave-65 契约矩阵：审批类控制请求可恢复（approval-resume）

> 工单：TUI 会话 `pa_01a1b122-b0de-7162-8c23-e29e27409417` 的 `turn-2e0eb0fe` 因工作区外 `Read`
> 触发 `ApprovalRequired`，被 `approval_resume_unavailable` 判死整轮（13 跳工具结果全丢）。
> 裁决：Lead｜状态：**frozen**（变更须 Lead 原地改写 + 文末流水）｜基线 `ebf50d64`（工作树另有
> 并行会话的 3 个未提交文件，严禁触碰/暂存）。
> 分级：**B**（跨 `runtime` / `dispatcher` / `control_plane` / `tui` 四模块，涉及权限语义但**不改**
> 权限判定本身；无 schema 迁移、无序列化编号变化）。

## 0. 根因（已复核）

审批类挂起被判死的唯一判据是 `turn_control::suspension_is_unrecoverable` 对所有 `Approval` 返回
`true`（`runtime/turn_control.rs:88-95`），因为当时没有「批准后恢复」的实现。同时 TUI 侧审批面板
早已具备（`tui.rs:9566+`、`authorize_path` 于 `tui.rs:9638`），但 `list_pending_asks` 只读 Ask 持久化
store（`control_plane/ask_plan_commands.rs:27-62`），Approval 永远进不了面板 ⇒ 面板是死代码。
⇒ 修复 = 补齐「挂起可恢复 + 宿主可列出审批 + 批准/拒绝后收口续轮」三件事，**不动权限判定**。

## 1. 语义不变量（INV）

- **INV-1 安全**：未获批准前工具**绝不执行**；批准只写入 read 作用域路径授权
  （`store_metadata.path_authorizations.v1`），不扩大写权限。
- **INV-2 桌面/CLI 零回归**：`AgentRuntime` 默认 `host_approval_resume = false`，桌面（`src-tauri`）与
  CLI 行为逐字不变（仍 fail-fast `approval_resume_unavailable`）；本单**不改** `src-tauri/src/**`。
- **INV-3 无凭据外泄**：宿主投影**不得**包含 `nonce`；CAS 四元组仍由宿主内部持有。
- **INV-4 CAS 不放松**：`can_consume` 的重放/过期/跨会话/版本/nonce 判定逐字不变；拒绝路径走
  `cancel_control_request`（→ `Cancelled`），批准路径走 `approve_control_request`（→ `Consumed`）。
- **INV-5 冻结测试不可改**：`crates/pony-agent-core/tests/ask_resume_e2e.rs`、`tests/permission_fail_closed_contract.rs`
  严禁修改；`turn_control.rs` 内单测允许随语义迁移同步改写（必须在本文登记）。

## 2. 冻结契约

**C1 — 运行时能力位（唯一开关）**
- `AgentRuntime` 新增字段 `host_approval_resume: bool`（默认 `false`），`AgentRuntimeBuilder::host_approval_resume(bool)` 设置；
  `runtime/mod.rs` 唯一字面量构造点同步。
- `suspension_is_unrecoverable`：`Approval` 在 `host_approval_resume == true` **且**
  `pending_request.session_id.is_some()` **且** 该请求是**路径授权类**
  （`pending_request.options` 的 `approvalKind == "path_authorization"`）时返回 `false`；其余形态逐字不变。
  **R2 裁决**：非路径审批（`tool_rule_ask` / `external_requires_approval` / 写执行保守抬升，
  `approvalKind == "approval"`）**不**进入可恢复挂起，保持既有 fail-fast `approval_resume_unavailable` ——
  这类审批被消费后不产生任何豁免，续轮重试必再审批，放开只会制造「批准→续轮→再审批」无限循环；
  真正的非路径审批恢复需要 dispatcher 级一次性豁免（waiver），已登记 backlog（见
  `.dev-team/backlog-approval-resume-followups.md` F-1）。
- 结果：命中既有可恢复分支 ⇒ 只发 `turn:suspended`（`turn:failed` 恰 0 条），请求保持 `Pending`。
- TUI 侧 `AgentRuntimeBuilder::desktop().host_approval_resume(true)`（唯一消费点）。

**C2 — 审批请求带结构化上下文（不新增结构体字段）**
- `GovernedDispatcher::persist_pending_request`：`ApprovalRequired` 时 `options` 写
  `{"approvalKind":<成因>,"tool":<model 名>,"path":<原始 path>,"scope":"read"}`；**R3**：`approvalKind`
  必须由**审批成因**派生（`decision_source == "path_permission_requires_authorization"` ⇒
  `"path_authorization"`，其余成因一律 `"approval"`），**禁止**以「实参里是否带 path 键」判定——
  否则 `tool_rule_ask` / `external_requires_approval` 带 path 实参时会被误标为路径授权类，触发
  「批准→续轮→再审批」循环或 `authorize_path` 必败；无 path 的 `approval` 不带 `path` 键。
  `prompt` 改为中文可读文案（含工具名与路径，仅路径授权类才提“工作区外路径”）。
- `Interaction`（Ask）路径的 `ask_options_from_arguments` 语义逐字不变。

**C3 — 控制面审批读取与拒绝**
- `HostControlPlane::list_pending_approvals(session_id: &str) -> Value`：从 `ask_dispatcher.pending_requests()`
  过滤 `Approval` + `Pending` + 同 `session_id` + **属于路径授权类**（`is_path_authorization_approval()`，R3）+ **且未过期**（`now_ms <= expires_at_ms`，R2：过期请求
  已不可 CAS 消费，若继续列出会让面板每 500ms 重弹成活锁），投影
  `{requestId,version,requestKind:"approval",sessionId,runId,turnId,callId,descriptorId,prompt,options,expiresAtMs}`
  （**无 nonce**）；空/空白 scope 返回空数组（fail-closed）。判据抽成纯函数
  `approval_is_listable(&PendingControlRequest, now_ms) -> bool` 以便单测。
- `HostControlPlane::cancel_approval_request(request_id: &str, expected_version: u64) -> Result<Value,String>`：
  **先校验 `request_kind == Approval`（R2，fail-closed，禁止经审批取消面取消 Ask）**，再经
  `ControlRequestAuthorization::for_request(..)` + `dispatcher.cancel_control_request(..)` 转 `Cancelled`。

**C4 — TUI 审批闭环**
- `sync_pending_control_request`：**审批项优先、Ask 项在后**（R2：契约 C4 原文如此；实现曾反序），
  两路快照合并后展示；审批走既有 `permission_request_from_pending`，但授权路径**只能**取结构化
  `options.path`（新增 `PermissionRequest.approval_path: Option<String>` 字段承载；**禁止**用
  `command_or_path`——它在 options.path 缺失时回退 `callId`，会把 callId 当路径做 canonicalize 必败）。
- 批准（`y` / Enter 的 `AllowOnce|AllowSession`）：`approval_path` 为 `Some` 时 `authorize_path(path,"read")` →
  `approve_control_request` → 投递 `UiEvent::ApprovalResumeTurn { prompt }`；`approval_path` 为 `None`
  时**跳过**授权直接 approve（R2：防无 path 审批被永久拒批）；失败上中文提示、不静默。
  顺序保持「先授权后 CAS」，R2 记录其已知代价（CAS 失败会残留一条 read 授权）为 ADVISORY/backlog F-3。
- 拒绝（`n` / Esc）：`cancel_approval_request` → 投递 `UiEvent::ApprovalResumeTurn { prompt }`（拒绝文案）。
- 续轮：`UiEvent::ApprovalResumeTurn` 复用 AskResumeTurn 的基建（`TuiTurnSink` + `turn_loop_in_flight`），
  固定宿主文案、不产生用户气泡；批准/拒绝文案分别为常量。
- 决策逻辑抽成可测纯接缝 `apply_approval_decision(state, control_plane, choice) -> Option<String>`
  （返回续轮文案），由 `tui.rs` 单测覆盖（真实 control plane + 内存 store）。

**C5 — 红相验收（Test Agent 唯一写）**
`crates/pony-agent-core/tests/approval_resume_e2e.rs`（黑盒，mock provider TCP，照 `ask_resume_e2e.rs` 骨架）：
- **T1**：能力开 + 工作区外 `Read` ⇒ `turn:suspended` 恰 1、`turn:failed` 恰 0；`list_pending_approvals(session)`
  恰 1 且带 `path`；provider 只被请求挂起前那次（不回灌 `control_outcome_pending`）。
- **T2**：`authorize_path` + `approve_control_request` 后跑续轮 ⇒ 同一 `Read` 被放行（`Allow`），
  续轮 `turn:completed` 恰 1，且续轮请求体不含 `control_outcome_pending`。
- **T3**：`cancel_approval_request` 后 `approve_control_request` 必 Err（CAS 未破）；拒绝后请求不在待决列表。
- **T4**（INV-2）：能力关（默认）⇒ 终态 `turn:failed` 且 error 以 `approval_resume_unavailable` 开头，
  `turn:suspended` 恰 0。
- **R3 增补**：`approvalKind` 由 `decision_source` 派生（dispatcher）、`is_path_authorization_approval` 共享判据、列表只列路径授权类、`APPROVAL_STALE_NOTICE`；对应单测 6 例（dispatcher 3 + tool_runtime/control_plane 2 + tui 2 中新增项）。
- **R2 增补（单元级判据，不修改已冻结的 T1~T4）**：
  - `approval_plan`（TUI 纯函数）单测：投影含 `options.path` ⇒ `authorize == Some(path)`；缺失 ⇒ `None`；
    批准/拒绝文案非空且不同；`apply_approval_decision` 无句柄 ⇒ `None`。
  - `approval_is_listable` 单测：有效 Approval ⇒ true；`now > expires_at_ms` ⇒ false；`Interaction`/非 Pending ⇒ false。
  - `cancel_approval_request` 对 `Interaction` 请求必须 Err（kind 闸门）。
  - **已知验收盲区（R2 如实登记，进 backlog F-2）**：TUI 接线的「授权→批准→续轮」正向路径目前无
    自动化 e2e（`apply_approval_decision` 位于 bin，集成测试不可导入；bin 单测无法在不跑真实 turn 的
    前提下构造 Pending Approval）。核心语义由 `approval_resume_e2e.rs` T1~T4 覆盖，接线分支由上述
    纯函数单测覆盖，剩余风险登记 backlog。

## 3. 门禁

- 机器：`cargo check -p pony-agent-core --bins`、`cargo test -p pony-agent-core --test approval_resume_e2e`、
  回归 `cargo test -p pony-agent-core --lib turn_control` / `--test ask_resume_e2e` / `--test permission_fail_closed_contract`。
- 审查：≥2 路（A 安全/契约、B 状态流转/并发）+ 1 路 L3-P 六问。
- 可观测性：本次新增宿主读取接口与 UI 交互 ⇒ 收口跑 `audit-diff.sh --base ebf50d64`，WARN 逐条解释。

## 4. 变更流水

- 2026-11-06 Lead：初版冻结（B 级，wave-65）。
- 2026-11-06 Lead R2（审查后裁决）：范围收紧为**仅路径授权类**审批可恢复（C1）；补 TTL 列表过滤 + 取消 kind 闸门（C3）；
  C4 改用结构化 `options.path`（新增 `PermissionRequest.approval_path`）并恢复「审批优先展示」；C5 增补单元级判据与
  验收盲区登记；ADVISORY 全部转 `.dev-team/backlog-approval-resume-followups.md`。
- 2026-11-06 Lead R3（第二轮审查后裁决）：审批成因派生改为 `decision_source`（禁止按实参 path 派生）；审批列表只列路径授权类；审批专属失效文案。同轮以 tmux 真机复验（工作区外 Read → 面板 → Enter 批准 → 授权落盘 → 续轮读成功；Esc 拒绝 → 续轮改道）。
