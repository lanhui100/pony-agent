# 2026-05-28 Session 39 PA-018 Control Plane Stream Summary Retrieval Fallback

## 本轮目标

- 继续推进 `PA-018`
- 把 graph stream 主链里残留的原始 session summary fallback 收口到 retrieval boundary
- 补齐对应验证与任务文档回写

## 本轮改动

- 更新：
  - `src-tauri/src/agent/control_plane.rs`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `HostControlPlane.execute_graph_run_stream()` 在从事件流重建 `TurnResult` 时，如果 terminal event 没带 `session_summary`：
  - 现在会回退到 retrieval `session_context.summary`
  - 不再直接读取原始 `SessionSnapshot.summary`
- 还新增了一条直接单测：
  - `recording_turn_event_sink_uses_fallback_summary_when_terminal_event_has_none`
  - 这条测试专门守住“terminal event 无 summary 时，必须消费 fallback summary”这条行为
- 这让 graph stream 主链又少了一处对原始 session artifact 的直接依赖
- `PA-018` 在 `C. runtime 接入` 上多了一条更深层的 retrieval 消费证据，不再只停留在 prompt/build/planner/handoff 或前端 run 视图

## 验证

已通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib control_plane::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture
```

结果：

- `control_plane::tests` 共 `10` 条测试全部通过
- `graph::tests` 共 `11` 条测试全部通过

## 当前结果

- `PA-018` 的 retrieval boundary 已进一步进入 control-plane / graph-stream 主链
- 当前仍不能宣布 `PA-018` 完成交付，因为 capability / bridge 层与更完整的稳定事实来源仍未收口

## 下一步动作

1. 继续找出 capability / bridge 层里仍默认读原始 session/run/checkpoint 的入口
2. 继续扩展 `LongTermMemory` 的保守、显式、可审计稳定事实来源
3. 接近收口时，再做一轮正式 closeout audit

## 当前卡点

- graph stream 主链已进一步收口，但整条 capability / bridge 迁移链仍未完成
- 现有验证证据持续增强，但还不足以支撑最终关单
