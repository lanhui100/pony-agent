# PA-039 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-039-build-memory-write-hooks-and-persisted-side-effect-contract.md`
- `openspec/changes/add-memory-write-hooks-and-persisted-side-effect-contract/specs/memory-write-hooks-and-persisted-side-effect-contract/spec.md`
- `openspec/changes/add-memory-write-hooks-and-persisted-side-effect-contract/tasks.md`
- `src-tauri/src/agent/hooks.rs`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src-tauri/src/agent/execution_control.rs`

## 审核口径

只按 `PA-039` 当前任务卡与 delta spec 的完成边界判断：确认 `long-term memory write` 路径是否已经形成 `normalized write intent -> hook decision -> persisted evidence -> recovery/replay` 的真实合同，并且 deny/transform/evidence/checkout/reload 都可复核；不把未来通用 side-effect 扩展当作本卡关闭前置。

### 不在本审计内

- 通用 side-effect family 的进一步抽象
- `replace_long_term_memory(...)` 的后续扩展
- 是否把 `memory_write_hook_trace_records` 继续投影到更细粒度的 runtime-view/checkpoint 摘要

## 逐项结论

### A. memory-write hooks 已消费规范化写入意图，而非直接操作 session store

状态：`达成`

代码参考：

- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:195)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:1865)

判断：

memory-write hooks 已挂在规范化 write intent 上，不再依赖对 session store 的隐式 mutation。

### B. persisted_effect / replay_required 合同已进入 recovery 判定链

状态：`达成`

代码参考：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:2108)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:5209)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:5277)

判断：

`PersistedEffectEvidence` 已带 `source_history_node_id` 恢复锚点；当最新节点缺少足够 evidence 时，恢复路径会稳定降级到 `replay_required`，不依赖前端猜测。

### C. hook trace 已进入 session truth-source，并可随 reload / history checkout 读回

状态：`达成`

代码参考：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:112)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:1364)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:1726)

验证：

- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_hook_trace_records_roundtrip_through_store -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::history_checkout_restores_memory_write_hook_trace_records_from_selected_node -- --exact --nocapture`

判断：

memory-write hook 自身的决策轨迹已经正式进入 session truth-source，而不是只停留在运行时结果对象。

### D. allow / deny / transform / persisted-effect / replay-required 验收矩阵已具备可复核证据

状态：`达成`

验证：

- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::append_turn_persists_memory_write_evidence_for_explicit_note -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_evidence_roundtrip_through_store -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_guard_deny_blocks_persistence_and_memory_mutation -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_transform_patch_can_rewrite_persisted_memory_intent -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_projects_memory_write_evidence_from_session_snapshot -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_keeps_replay_required_when_latest_node_has_no_memory_write_evidence -- --exact --nocapture`

判断：

当前任务卡承诺的五类路径都已经拿到后端定向验证证据。更广义 conflict/failure-policy family 属于后续扩展，不再阻断本卡关闭。

## 验证备注

- 本轮重新执行的 transform / trace roundtrip / checkout / checkpoint / replay-required 测试全部通过。
- `cargo check --manifest-path src-tauri/Cargo.toml --tests` 与 `cargo test --manifest-path src-tauri/Cargo.toml --test session_regression -- --nocapture` 现已通过；此前 `session_regression` 缺失 `capability_bridge` fixture module 的编译断点已一并修复。
- Windows 环境仍有 incremental finalize `os error 5` warning，但未影响通过判定。

## 最终裁定

`PA-039` 已满足任务卡与 delta spec 的完成边界，可以从 `In Progress` 更新为 `Done`。

关闭理由：

1. `long-term memory write` 路径已形成 `intent -> hook -> evidence -> recovery` 的真实闭环。
2. `memory_write_evidence` 与 `memory_write_hook_trace_records` 已完成 session snapshot、history checkout、file roundtrip 与 checkpoint recovery 的读回闭环。
3. `persisted_effect / replay_required` 的最小证据与安全降级合同已可验证，不再依赖隐式恢复推断。
