# Proposal: Split runtime ownership and unblock turn/read paths

## Why

当前 `AgentRuntime` 的 ownership 仍然混在一个全局运行时对象里，turn 执行和 session/read-plane 查询共享过大的状态边界。即使前端已经做了缓存优先，真正的多会话并发和后台查询解耦依旧受制于全局锁模型。

## Scope

- 识别 `AgentRuntime` 的 state ownership 分层
- 将 session/read-plane 与 turn execution 从结构上解耦
- 为 async provider 和 per-session task 模型提供前置边界

## Non-goals

- 本 change 不直接完成 provider async 化
- 本 change 不直接完成 per-session task orchestration

## Cleanup

- 移除旧的 `Mutex<AgentRuntime>` 全局锁模式和关联 dead code
- 更新依赖旧锁模型的 `#[cfg(test)]` 测试
