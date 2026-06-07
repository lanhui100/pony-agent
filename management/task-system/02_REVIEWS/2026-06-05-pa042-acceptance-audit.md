# PA-042 Acceptance Audit

## 审核范围

- [management/task-system/03_TASKS/PA-042-build-session-control-audit-surface-and-history-evidence-summary.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-042-build-session-control-audit-surface-and-history-evidence-summary.md)
- [openspec/changes/archive/2026-06-05-add-session-control-audit-surface-and-history-evidence-summary/specs/session-control-audit-surface-and-history-evidence-summary/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-session-control-audit-surface-and-history-evidence-summary/specs/session-control-audit-surface-and-history-evidence-summary/spec.md>)
- [openspec/changes/archive/2026-06-05-add-session-control-audit-surface-and-history-evidence-summary/tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-session-control-audit-surface-and-history-evidence-summary/tasks.md>)
- [docs/architecture/session-control-plane-and-audit-surface.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [src/types/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)
- [src/components/HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)
- [tests/runtime-store.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/runtime-store.spec.ts)
- [tests/HomeSessionSidebar.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeSessionSidebar.spec.ts)

## 审核口径

只按 `PA-042` 当前任务卡与 delta spec 的完成边界判断：确认 `Session Control Plane audit surface v1` 是否已经把 `history checkout / restore / fork / switch` 四类 control action 收口为统一 summary read-model，并满足 persisted projection、reload roundtrip、truth-source guardrail 与前端 summary-first explainability 的完成要求。

### 不在本审计内

- `stop / continue / resume / replay` 的 run-control summary family
- `PA-037` 既有按钮布局、disabled reason 与状态语言重做
- 新的 history command、workflow scheduler 或 replay backend command

## 逐项结论

### A. Canonical history-control audit summary contract

状态：`达成`

发现：

- `session.rs` 已定义 `HistoryStateAuditActionSummary / HistoryStateAuditCurrentContext / HistoryStateAuditSummary`，并通过 `build_history_state_audit_summary(...)` 统一生成 summary。
- summary 已显式拆分为：
  - `action evidence summary`
  - `current context projection`
- `action` 已覆盖当前 spec 要求的核心字段：`status / source_family / command_kind / boundary / result_kind / summary / elapsed_ms / blocked / degraded / evidence_id / observed_at_ms`。
- `PA-042 v1` 只面向 `checkout / restore / fork / switch` 四类 history-control 命令，没有吸收 run-control summary family。

证据：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [src/types/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)
- [docs/architecture/session-control-plane-and-audit-surface.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md)

判断：

`PA-042` 已形成稳定的 summary contract，而不是继续依赖零散 evidence 字段让前端自行拼装。

### B. Persisted projection / runtime view / command response 一致性

状态：`达成`

发现：

- `SessionSnapshot` 已持久化 `history_state_audit_summary`。
- `SessionRuntimeView` 与四类 history-control response 都直接投影同一口径 summary。
- `control_plane.rs` 中 `project_history_state_audit_summary(...)` 已作为统一投影入口，避免各个读面重算结论。
- `runtime.ts` 已把 summary 接入：
  - `loadSessionState(...)`
  - `applySessionSnapshot(...)`
  - `checkoutHistoryNode(...)`
  - `restoreBranchHead(...)`
  - `forkHistoryNode(...)`
  - `switchHistoryBranch(...)`

证据：

- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)

验证：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib history_checkout_response_and_runtime_view_share_same_history_state_evidence_projection -- --nocapture
```

判断：

summary 已具备 “persisted truth-source -> snapshot/runtime view/response -> frontend store” 的统一投影闭环。

### C. Reload roundtrip 与 missing evidence 负向路径

状态：`达成`

发现：

- reload 后若 persisted evidence 仍在，summary 会随 `SessionSnapshot` 一起读回。
- 若底层 evidence 缺失，系统只表现为 `missing`，不会反推 restore 成功、branch 切换成功或 workspace rollback 成功。
- `missing` 语义仅影响审计读面，不影响 cursor / branch / rollback 真值。

证据：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)

验证：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib missing_history_state_evidence_does_not_reconstruct_restore_conclusion_after_reload -- --nocapture
```

判断：

`PA-042` 最关键的 reload guardrail 已经成立：summary 缺失不会变成新的仲裁输入。

### D. Truth-source guardrail 与 degrade/non-regression

状态：`达成`

发现：

- summary 只作为 read-model，不会改写既有 `HistoryCursor`、branch head 或 rollback 真值。
- degraded 结论继续来自既有 restore truth-source；summary 只投影该结论。
- 当前上下文展示与动作证据已分层，避免把 `visibleNodeId / branchHeadNodeId` 误当成动作结果。
- `runtime-store` 里保留了必要兼容字段映射，但主链路已明确优先消费 summary contract。

证据：

- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [src/components/HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)

判断：

`summary-first` 已经成立，但没有突破既有 truth-source 边界。

### E. Frontend explainability 主消费切换

状态：`达成`

发现：

- `HomeSessionSidebar` 已优先消费 `latestHistoryStateAuditSummary`。
- UI 已拆分展示：
  - `最近动作证据`
  - `当前上下文（非动作证据）`
- `tests/HomeSessionSidebar.spec.ts` 已覆盖：
  - success
  - missing
  - degraded
  - blocked
- `tests/runtime-store.spec.ts` 已覆盖 summary hydration 与 history-control response 写回 `latestHistoryStateAuditSummary`。

验证：

```powershell
npm run test:unit -- --run tests/HomeSessionSidebar.spec.ts
npm run test:unit -- --run tests/runtime-store.spec.ts
```

判断：

前端主展示已经切到 summary-first，不再依赖私有推理链作为主真相源。

### F. 编译与任务完成度

状态：`达成`

发现：

- Rust 编译门已通过。
- OpenSpec tasks 除回写项外已全部完成；本次 acceptance audit 与 closeout 会补齐最后一项。

验证：

```powershell
cargo check --manifest-path src-tauri/Cargo.toml --lib
```

判断：

`PA-042` 的代码、验证和任务系统收口条件已经齐备。

## 最终裁定

`PA-042` 已满足当前任务卡与 delta spec 的完成边界，可以从 `In Progress` 更新为 `Done`。

关闭理由：

1. `Session Control Plane audit surface v1` 已对 `history checkout / restore / fork / switch` 建立统一 summary read-model。
2. summary 已完成 persisted projection、reload roundtrip、runtime view / response 一致性与前端 summary-first 主消费切换。
3. `missing evidence` 与 truth-source non-regression 两条最高风险 guardrail 已通过显式定向验证。
4. 当前剩余工作只涉及归档与后续 run-control 扩展，不再构成 `PA-042` 当前 scope 的完成阻断。
