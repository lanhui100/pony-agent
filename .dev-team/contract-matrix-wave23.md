# wave-23 契约冻结矩阵：调度器预算防御、同步取消探测与 PlanStore 实例收敛

## 1. 现状与诊断
- PlanStore 脑裂：`HostControlPlane` 内部构建了一个 `PlanStore`，而 `build_governed_executor` 内部又独立构造了一个 `PlanStore::new()`，导致通过宿主控制面创建的 Plan 与 Agent 模型在 `plan_control` 中管理的 Plan 处于两套完全隔离的内存世界。
- 同步轮次无法取消：`turn_sync.rs` 在长任务模型多跳迭代中没有轮询 `should_cancel_turn`，导致在执行复杂单轮任务时外部中断信号无法有效传递，造成主线程僵死卡死。

## 2. 目标契约 (C1 ~ C2)

### C1: PlanStore 单例共享契约
- `build_governed_executor_with_plan_store` 或在构建参数中接受可选的 `Option<PlanStore>`；当宿主传入共享的 `PlanStore` 时，`PlanControlHandler` 必须绑定至该同一实例。
- 宿主与 Agent 侧工具对计划的创建、状态更新保持原子一致。

### C2: 同步路径取消探测
- 在 `turn_sync.rs` 的循环体内部（`while completed_hops < max_hops`），在调用下一次模型或执行工具前，检查会话或运行时的取消状态，若已取消则立即中断并返回已取消错误。
