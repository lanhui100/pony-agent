# PA-065 拆分 runtime ownership 并解除 turn 与读路径互锁

## 状态
- Status: `Ready`
- Priority: `P1`
- Owner: `待定`

## 依赖
- 前置：`PA-064`
- 后续为：`PA-066`、`PA-067`、`PA-068`

## Canonical Spec
- `openspec/changes/split-runtime-ownership-and-unblock-turn-read-paths/specs/session-control-surface-and-feedback-loop/spec.md`

## OpenSpec Change
- `openspec/changes/split-runtime-ownership-and-unblock-turn-read-paths/`

## 背景
当前 `HostControlPlane` 仍通过全局 `Mutex<AgentRuntime>` 把 turn 执行与 session/read-plane 查询绑在一起。即使前端已改成缓存优先，真正的多会话并发与后台读写解耦仍被大锁限制。

## 目标
1. 识别并拆分 `AgentRuntime` 中的 ownership 边界
2. 解除 `turn execution` 与 `session/list/read-plane` 的全局互锁
3. 为后续 async provider 与 per-session task 模型建立稳定接口前提

## In Scope
- `AgentRuntime` / `HostControlPlane` 的状态职责切分
- `SessionStore`、`GraphRunStore`、provider/tool/planner 相关 state 的 ownership 识别
- 去掉“整轮 turn 期间持有全局 runtime 锁”的关键路径
- 明确只读查询路径与执行路径的隔离合同

## Out of Scope
- provider 全量 async 化
- per-session task orchestration 最终接线
- 前端 UI 渲染优化

## 验收标准
- session read-plane SHALL NOT 与整轮 turn execution 共享同一把全局运行时锁
- `list_sessions` / `load_session_runtime_view` / `load_retrieved_context` SHALL 有独立的可演进边界
- 后续 `PA-066` / `PA-067` 能在该 ownership 模型上并行推进而不返工
- 旧的 `Mutex<AgentRuntime>` 全局锁路径 SHALL 被移除或降级为窄范围专用锁
- 原有依赖全局锁模式的 `#[cfg(test)]` 测试 SHALL 被迁移

## 下一步动作
1. 画出当前 runtime state ownership 图
2. 标注哪些字段必须 session-local、run-local、global
3. 先改 control plane 持锁边界，再补定向测试

## 断点续跑提示
- `crates/pony-agent-core/src/agent/control_plane.rs`
- `crates/pony-agent-core/src/agent/runtime/mod.rs`
- `crates/pony-agent-core/src/agent/session.rs`
- `crates/pony-agent-core/src/agent/graph.rs`

## 当前进展
- 已完成

## 完成摘要
- `HostControlPlane` 新增 `sessions_rwlock: Arc<RwLock<SessionStore>>` 字段
- `AgentRuntime.sessions` 从 `SessionStore` 改为 `Arc<RwLock<SessionStore>>`
- `AgentRuntime::sessions_handle()` 提供 `Arc` 共享
- 16 个读面方法从 `self.runtime.lock()` 迁移到 `self.sessions_rwlock`（读方法用 `.read()`，含惰性初始化副作用的方法用 `.write()`）
- `SessionBackend` trait 增加 `+ Sync` 约束使 `RwLock<SessionStore>` 可编译
- 3 轮并行智能体审核后调优：`.unwrap()` 改为 `.expect()`，锁消息统一
- 清理与收口已完成
