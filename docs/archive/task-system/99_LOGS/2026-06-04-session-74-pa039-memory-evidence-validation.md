# 2026-06-04 Session 74 - PA-039 Memory Evidence Validation

## 主题

- 验证 `PA-039` 第一轮真实接线是否已经形成可持久化的 memory-write evidence 闭环
- 确认此前超时现象是否来自实现阻塞，还是仅由冷缓存与测试过滤器误用导致

## 本轮结论

1. `PA-039` 当前实现并未卡在逻辑阻塞
   - 初始超时主要来自两点：
     - 使用自定义 `CARGO_TARGET_DIR` 导致冷编译成本过高
     - 使用了未命中完整测试名的过滤器，实际只完成了编译，没有真正执行目标测试
   - 改为默认 target，并使用完整测试名后，目标测试均在数十秒内完成

2. memory-write evidence 最小闭环已被真实验证，并继续推进到 history-node 级恢复判定与可控写入
   - `agent::session::tests::append_turn_persists_memory_write_evidence_for_explicit_note` 通过
   - 证明 `append_turn -> update_long_term_memory_from_user_message(...)` 已能生成 `memory_write_evidence`，并把 evidence 锚定到新提交的 history node
   - `agent::session::tests::memory_write_evidence_roundtrip_through_store` 通过
   - 证明 `memory_write_evidence` 已能随 `FileSessionBackend` 持久化并在 reload 后读回，同时保留 history-node 锚点
   - `agent::session::tests::memory_write_guard_deny_blocks_persistence_and_memory_mutation` 通过
   - 证明 memory-write guard deny 已能真实阻止 long-term memory mutation 与 evidence 持久化，而不是只停在结果对象
   - `agent::session::tests::memory_write_transform_patch_can_rewrite_persisted_memory_intent` 通过
   - 证明 transform patch 已能真实改写待持久化的 memory intent，而不是只修改 summary 文本
   - `agent::control_plane::tests::lifecycle_boundary_checkpoint_projects_memory_write_evidence_from_session_snapshot` 通过
   - 证明与当前 lifecycle boundary 匹配的 memory-write evidence 会投影到 `ExecutionCheckpoint.persisted_effect_evidence`，并把 recovery mode 收敛到 `persisted_effect`
   - `agent::control_plane::tests::lifecycle_boundary_checkpoint_keeps_replay_required_when_latest_node_has_no_memory_write_evidence` 通过
   - 证明旧历史节点上的 memory-write evidence 不会误升级最新 boundary 的 recovery 判定；当最新节点没有匹配 evidence 时，checkpoint 仍稳定回到 `replay_required`

3. `PA-039` 的下一步焦点已收敛
   - 不是继续怀疑 `session.rs` 这根真线
   - 而是继续把 `memory_write_evidence` 从 history-node 级 recovery 判定推进到更严格的 turn / recovery 口径
   - allow/deny/transform 已有第一轮真实行为验证，下一步补 conflict/failure policy/traceability 的完整 contract 矩阵

## 执行命令

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --list | rg "append_turn_persists_memory_write_evidence_for_explicit_note|memory_write_evidence_roundtrip_through_store"
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::append_turn_persists_memory_write_evidence_for_explicit_note -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_evidence_roundtrip_through_store -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_guard_deny_blocks_persistence_and_memory_mutation -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_transform_patch_can_rewrite_persisted_memory_intent -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_projects_memory_write_evidence_from_session_snapshot -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_keeps_replay_required_when_latest_node_has_no_memory_write_evidence -- --exact --nocapture
cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet
npm run test:unit -- --run tests/runtime-store.spec.ts
```

## 验证结果

- 六条目标验证均通过
- 前端 `runtime-store` 定向测试通过：`55 passed`
- 期间仍出现既有的 Windows incremental finalize `os error 5` warning，但未影响测试通过判定
- roundtrip 测试日志已确认 sessions 文件发生加载与保存
- control-plane 测试已确认 lifecycle boundary checkpoint 与 runtime view 都能读到 `persisted_effect_evidence`
- 当前新增的 `sourceHistoryNodeId` 已经进入 session snapshot、store roundtrip、checkpoint 投影链，并参与 recovery mode 的严格过滤判定
- 当前新增的 `MemoryWriteHookExecutor / value_text / kind+content intent fields` 已进入真实写入路径，并通过 deny/transform 行为验证

## 任务系统回写

- 已更新 [PA-039](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-039-build-memory-write-hooks-and-persisted-side-effect-contract.md)
- 已更新 [Task Board](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- 已更新 [Dashboard](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
- 已更新 [OpenSpec Tasks](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-memory-write-hooks-and-persisted-side-effect-contract/tasks.md)
