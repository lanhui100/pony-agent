## Context

当前系统中：

- `session.rs` 已可记录 `turn_trace_history`
- `execution_control.rs` 的 checkpoint 仍是进程内 registry
- `control_plane.rs` 已暴露 history checkout / restore / branch 相关读写面
- `runtime.ts` 仍包含大量 trace fallback 与 hydration 推导逻辑

本变更要做的是把这些能力的职责边界正式化，不在本卡内一次性实现所有恢复能力。

## Goals / Non-Goals

**Goals**

- 定义后端 trace persistence 是事实源
- 定义 runtime checkpoint 与 recovery checkpoint 的边界
- 定义 history checkout 与 workspace rollback 的结果口径
- 定义前端何时只展示、何时允许有限 fallback

**Non-Goals**

- 立即实现完整 provider-level 断点续传
- 立即重写全部 history graph UI
- 立即删除所有前端 fallback 代码

## Decisions

### 1. Trace 持久化事实源固定在后端 session store

原因：

- trace 指标与 timeline 必须能跨应用重启保留
- 前端 localStorage 只能作为浏览器预览或开发兜底，不得成为桌面 runtime 的主事实源

### 2. Checkpoint 必须区分运行中与可恢复

做法：

- `runtime checkpoint`：服务 stop/cancel/pause
- `recovery checkpoint`：服务 reload/resume/replay
- `recovery checkpoint` 额外显式给出 `recoveryMode`
  - `replay_required`
  - `persisted_effect`

原因：

- 运行中 checkpoint 与可恢复 checkpoint 的语义完全不同
- `resumable / replayable` 仍不足以表达“是否允许直接 resume 旧运行”
- 当前端 `runState` 与 recovery 合同冲突时，必须以后者为准

### 3. 恢复结果必须显式表达降级

做法：

- transcript-only
- transcript+workspace
- degraded_to_transcript_only

原因：

- 避免 UI 或调用方误以为 workspace 已经成功回滚

### 4. 前端逐步退出 trace 事实拼装角色

做法：

- 优先消费后端 trace/history/checkpoint 读面
- `PA-032` 只负责 persisted trace/recovery truth 与 hydration/reload 语义
- 对 legacy 数据保留有限兼容，但不能继续扩展新一轮隐式 trace/recovery 推导

### 5. 有限 fallback 必须显式收口

做法：

- 桌面运行时只要后端 persisted snapshot 可用，就以后端为准
- fallback 只允许作为非 canonical UI 辅助态
- fallback 不得生成、覆盖或重算 canonical trace metrics
- 允许 fallback 的场景仅限 `backend unavailable`、`browser preview` 或后续明确列出的受控环境

## Implementation Outline

1. 明确 persistence / recovery data model
2. 收口宿主读面返回的状态对象
3. 压缩前端 runtime store 的推导面，并让恢复合同优先于旧 `runState` 猜测
4. 建立恢复与降级测试矩阵

## Verification Strategy

- Rust 测试覆盖 trace roundtrip、checkpoint 语义区分、history restore 降级口径
- 前端 store 测试覆盖重启后 hydration 与恢复展示保真
- 任务卡与 review 文件记录验收结论
