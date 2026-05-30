# PA-015 抽离宿主控制面与统一控制命令

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## 目标
在 graph/runtime 主线之外，并行把当前主要依赖 Tauri command/event 的宿主接入面抽成统一控制平面，让 Tauri、HTTP/SSE、CLI 后续都复用同一组 run / turn / session / telemetry 命令与查询模型。

## 输出
- host control plane 第一版接口
- 宿主命令矩阵（turn / run / session / checkpoint / telemetry）
- adapter 复用约束
- run / turn 查询与 inspect 读取草案
- 多宿主最小事件与响应模型

## 验收标准
- adapter 不承载独立调度逻辑，只负责入口翻译与事件投递
- 同一套 core 命令可被 Tauri、HTTP/SSE、CLI 复用
- 宿主能读取 run / turn / session 的最小 inspection 结果，而不直连 runtime 内部实现
- 文档明确本卡不等于“完整远程服务化”或“完整控制台 UI”
- `PA-012 ~ PA-014` 不需要为某个单独宿主重写 graph/runtime 逻辑

## 完成情况
- 已新增 `HostControlPlane`，统一持有 `AgentRuntime` 与 `ExecutionControlRegistry`
- 已收口统一命令/查询模型：
- `RunTurnCommand`
- `StartTurnStreamCommand`
- `StopTurnCommand`
- `DeleteSessionCommand`
- `ExecutionCheckpointQuery`
- `SessionSnapshotQuery`
- `HostInspectionQuery`
- 已新增统一 inspection 读面：`inspect() -> HostInspectionSnapshot`
- Tauri 命令层已收窄为薄入口，流式启动下沉到 `tauri_adapter::spawn_turn_stream(...)`
- SSE adapter 已改为复用同一 control plane，而不是绕回 Tauri 专属逻辑

## 验证
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`

## 后续衔接
- 后续 HTTP/SSE / CLI 宿主可继续在本卡控制面之上扩展，而不重写 graph/runtime 逻辑
- `PA-013 / PA-014` 新增 run/goal 级命令时，也应优先挂到 control plane，而不是直接膨胀宿主入口

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-007-split-core-adapters.md`
- `management/task-system/03_TASKS/PA-010-build-runtime-loop-and-stop-conditions.md`
- `docs/architecture/overview.md`
- `src-tauri/src/lib.rs`
- `src-tauri/src/tauri_adapter.rs`
- `src-tauri/src/sse_adapter.rs`
