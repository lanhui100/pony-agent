# PA-043 Acceptance Audit

## 审核范围

- [management/task-system/03_TASKS/PA-043-build-run-control-audit-surface-and-summary-first-explainability.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-043-build-run-control-audit-surface-and-summary-first-explainability.md)
- [openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md>)
- [openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/specs/run-control-audit-surface-and-summary-first-explainability/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/specs/run-control-audit-surface-and-summary-first-explainability/spec.md>)
- [openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/tasks.md>)
- [docs/architecture/session-control-plane-and-audit-surface.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [src/types/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)
- [src/components/HomeWorkspace.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeWorkspace.vue)
- [src/components/HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)
- [tests/runtime-store.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/runtime-store.spec.ts)
- [tests/HomeWorkspace.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeWorkspace.spec.ts)
- [tests/HomeSessionSidebar.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeSessionSidebar.spec.ts)

## 审核口径

只按 `PA-043` 当前任务卡与 delta spec 的完成边界判断：确认 `Run Control audit surface v1` 是否已经把 `stop / continue / resume / replay(start)` 收口为统一 summary read-model，并满足 persisted projection、reload/read-plane 一致性、truth-source guardrail 与前端 summary-first explainability 的完成要求。

### 不在本审计内

- `PA-037` 已成立的按钮编排、disabled reason 与状态语言重做
- 新的 graph run command、scheduler 能力或 workflow-level orchestration
- history-control summary family；该范围继续由 `PA-042` 承接

## 逐项结论

### A. Canonical run-control audit summary contract

状态：`达成`

发现：

- `session.rs` 已定义 `RunControlAuditActionSummary / RunControlAuditCurrentContext / RunControlAuditSummary`，并通过 `build_missing_run_control_audit_summary()` 固定 `missing` 形态。
- summary 顶层已固定拆成：
  - `action_evidence_summary`
  - `current_context_projection`
- `action_evidence_summary` 已覆盖当前 spec 要求的核心字段：`status / source_family / command_kind / boundary / result_kind / summary / target_summary / elapsed_ms / blocked / degraded / evidence_id / observed_at_ms`。
- `control_plane.rs` 已把 `start_reason` 明确投影为 `replay_from_checkpoint / restart_from_checkpoint`，并在普通首轮 `start_graph_run_stream` 时返回 `missing` summary，而不是误吸收到 run-control family。

证据：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [src/types/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)

判断：

`PA-043` 已形成独立、刚性的 run-control summary contract，而不是继续依赖 submission plan / checkpoint / boundary evidence 让前端自行拼装。

### B. Persisted projection / runtime view / command response 一致性

状态：`达成`

发现：

- `SessionSnapshot` 已持久化 `run_control_audit_summary`。
- `SessionRuntimeView`、`GraphRunControlResponse` 与 `GraphRunStreamStartResponse` 都直接投影同一口径 summary。
- `control_plane.rs` 中 `project_run_control_audit_summary(...)` 已作为统一投影入口，避免 snapshot/runtime view/response 各自重算结论。
- `runtime.ts` 已把 summary 接入：
  - `loadSessionState(...)`
  - `applySessionSnapshot(...)`
  - `stopTurn(...)`
  - `submitTurn(...)` 的 `start / continue / resume`

证据：

- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)

验证：

```powershell
$env:CARGO_INCREMENTAL='0'
$env:CARGO_TARGET_DIR='C:/Users/HUAWEI/Documents/pony-agent/.cargo-target-pa043'
cargo test --manifest-path src-tauri/Cargo.toml control_plane --no-run
```

判断：

summary 已具备 “persisted truth-source -> snapshot/runtime view/response -> frontend store” 的统一投影闭环。

### C. Reload roundtrip 与 missing evidence 负向路径

状态：`达成`

发现：

- reload 后若 persisted evidence 仍在，summary 会随 `SessionSnapshot` 与 `SessionRuntimeView` 一起读回。
- `build_missing_run_control_audit_summary()` 已固定 `missing` 语义，不会在 evidence 缺失时伪造 `continue / resume / replay` 已成功的结论。
- `runtime-store` 已覆盖：
  - runtime view summary hydration
  - runtime view 优先于 snapshot fallback
  - default start plan 下 `missing` summary 不会复活活动 run id

证据：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [tests/runtime-store.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/runtime-store.spec.ts)

验证：

```powershell
npm run test:unit -- --run tests/runtime-store.spec.ts
```

判断：

`PA-043` 最关键的 reload guardrail 已经成立：summary 缺失只会表现为 `missing/unavailable`，不会变成新的 run-control 仲裁输入。

### D. Truth-source guardrail 与 replay/start 语义分离

状态：`达成`

发现：

- summary 只作为 read-model，不会改写既有 `submission plan / execution checkpoint / graph run phase` 真值。
- replay/restart 结论继续来自既有 checkpoint / submission-plan 仲裁；summary 只投影该结论。
- `project_run_control_start_reason(...)` 已把 `start_graph_run_stream` 分成：
  - 普通首轮启动：不进入 run-control summary
  - `replay_from_checkpoint`
  - `restart_from_checkpoint`
- resume 路径已补显式断言，证明 `resume_graph_run_stream` 会生成 run-control summary，而不是落回默认 missing 形态。

证据：

- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [tests/HomeWorkspace.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeWorkspace.spec.ts)

判断：

最容易漂移的 replay/start 语义边界已经固定下来，summary 没有突破既有 truth-source 边界。

### E. Frontend summary-first explainability

状态：`达成`

发现：

- `HomeWorkspace` 已优先消费 `latestRunControlAuditSummary`，只在 summary 缺席时回退到旧字段。
- `HomeSessionSidebar` 已优先消费 `latestRunControlAuditSummary`，并把 run-control explainability 保持在既有展示位内，没有回流成 `PA-037` UI 重设计。
- 前端测试已覆盖：
  - resume CTA
  - replay/start CTA
  - stop summary explainability
  - `summary-first` hydration
  - “只换数据源、不改布局/CTA” non-regression

证据：

- [src/components/HomeWorkspace.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeWorkspace.vue)
- [src/components/HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)
- [tests/HomeWorkspace.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeWorkspace.spec.ts)
- [tests/HomeSessionSidebar.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeSessionSidebar.spec.ts)

验证：

```powershell
npm run test:unit -- --run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts
```

判断：

前端主展示已经切到 summary-first，但没有突破 `PA-037` 已成立的结构与交互边界。

### F. 验证与环境结论

状态：`达成`

发现：

- 前端 `runtime-store / HomeWorkspace / HomeSessionSidebar` 定向回归全部通过。
- Rust `control_plane --no-run` 已通过独立 `target` 目录编译验证。
- 新补的 run-control Rust 定向测试中：
  - `graph_run_stream_can_start_continue_and_resume` 已通过
  - `ordinary_start_graph_run_stream_does_not_enter_run_control_summary` 的实现问题已修正
- Windows 本机曾出现 `artifact directory / package cache` 锁竞争与 `LNK1201` PDB 写入冲突；改用独立 `CARGO_TARGET_DIR` 后，可区分出这属于本机链接/锁噪音，而不是 `PA-043` 逻辑错误。

判断：

`PA-043` 的代码、验证和任务系统收口条件已经齐备；剩余只涉及完成态回写，而不再构成 scope 阻断。

## 最终裁定

`PA-043` 已满足当前任务卡与 delta spec 的完成边界，可以从 `In Progress` 更新为 `Done`。

关闭理由：

1. `Run Control audit surface v1` 已对 `stop / continue / resume / replay(start)` 建立统一 summary read-model。
2. summary 已完成 persisted projection、runtime view / response 一致性、reload/hydration 与前端 summary-first 主消费切换。
3. 普通首轮 `start_graph_run_stream` 不进入 run-control summary 的 guardrail 已写入后端投影与定向测试。
4. 当前剩余工作只涉及归档与后续扩展，不再构成 `PA-043` 当前 scope 的完成阻断。
