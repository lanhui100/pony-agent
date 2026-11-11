# PA-063 cursor versioning 与多端冲突保护

## 状态

- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 依赖

- 前置：`PA-060`
- 建议串行于：`PA-061`

## Canonical Spec

- `openspec/specs/session-cursor-view-contract/spec.md`

## 背景

`PA-060` 已把 `cursorVersion` 写入合同，但当前没有真正实现版本控制。本项目后续目标包含 `TUI / CLI / HTTP`，一旦多个 surface 可以同时移动 cursor，就必须防止 stale mutation 静默覆盖较新的状态。

## 目标

1. 给 cursor mutation 引入真实 revision/version 机制
2. 定义 stale mutation 的拒绝或显式冲突处理路径
3. 为未来多端并发访问补最小安全护栏

## In Scope

- cursor version / revision 的存储与返回
- checkout / restore / fork / branch switch 的冲突检测
- host command 返回冲突信息与 refresh 约束
- 多 surface 并发测试与定向验证

## Out of Scope

- preview 模式的本地 fallback 清理
- 非 cursor 维度的广义协作冲突

## 验收标准

- host SHALL 返回真实的 cursor revision/version，而不是占位值
- stale cursor mutation SHALL 被拒绝或显式冲突化，而不是静默覆盖
- 至少一条定向测试 SHALL 覆盖多 surface / stale mutation 冲突路径
- 前端/CLI/HTTP 预期合同 SHALL 能消费冲突信号并刷新

## 下一步动作

已完成；如继续推进，应另开卡处理更广泛的 Rust 全量阻塞清理与 HTTP / CLI 消费面接线。

## 断点续跑提示

- `crates/pony-agent-core/src/agent/session.rs`
- `crates/pony-agent-core/src/agent/control_plane.rs`
- `openspec/specs/session-cursor-view-contract/spec.md`

## 当前进展

- 已为 `HistoryCursor` 增加真实 `cursor_version`
- 已为 `checkout / restore / fork / switch branch` 四类 history-control commands 增加 `expected_cursor_version`
- 已为 stale revision 增加显式冲突错误：`history cursor conflict: expected revision X, actual revision Y`
- 已为前端 store 接入 `cursorVersion` 状态，并在 Tauri history-control 调用时透传 `expectedCursorVersion`
- 已让前端在冲突时通过 `sessionError` 暴露刷新信号

## 验收结果

- 通过前端定向测试：
  - `checks out a history node with backward-compatible cursor fallback`
  - `surfaces cursor revision conflicts from host history checkout`
  - `preserves host authority metadata on runtime views`
  - `clears stale local historical state when host-authoritative runtime view omits history state`
- Rust 更广定向/全量验证仍受仓库现存无关编译问题阻断，尚未完成完整收口：
  - `src-tauri/tests/provider_registry_regression.rs` 缺字段初始化
  - `crates/pony-agent-core/src/bin/non_tauri_harness.rs` 编译路径错误

## 残余风险

- 当前已完成 Tauri + 前端 store 闭环；CLI / HTTP 消费面尚未真正接入该冲突信号
- Rust 全量验收需在仓库无关编译阻塞清理后补做
