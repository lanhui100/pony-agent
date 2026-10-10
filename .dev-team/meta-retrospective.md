# Wave 8 元框架复盘（Sentry 问题收集下沉与 Agent Trace 架构解耦）

> 交付：Sentry 问题收集从 Agent 核心解耦并下沉至 Harness 宿主层、ADR 0025 制定与代码收敛
> 范围：ADR-0025 编写、架构边界梳理、Agent Core 内部业务打点清理、前后端全量回归与元复盘

## 1. 通信拓扑与信噪比
- **协作模型**：严格遵循 `dev-team` 编排机制，由 Lead 统一拆解 DAG 任务，Protocol Agent 约束边界，Test Agent 与 Executor 专职分工。
- **信噪比控制**：避免主观臆断，所有指令通过 TaskBoard 状态流转并配合物理测试收据验证。

## 2. 门禁穿透与误杀率
- **物理机器收据**：
  - `cargo check -p pony-agent-core`：Exit Code 0；
  - `cargo test -p pony-agent-core --test ponysentry_trace_test`：Exit Code 0 (7/7 passed)；
  - `npm test`：Exit Code 0 (62 文件通过，724 passed)。
- **真实门禁执行**：未放行任何无断言测试，所有修改后的逻辑均通过全链路测试覆盖。

## 3. 分工契约与隔离有效性
- **ADR-0025 确立核心原则**：Agent Core 仅负责 Trace 聚合、Token 消耗统计与业务语义流转，通用 Crash、Panic 及基础设施异常收集由宿主/Harness 层统一兜底。
- **代码收敛**：移除了 `turn_flow.rs`、`provider/mod.rs`、`turn_persist.rs` 中直接侵入式调用底层 Sentry Issue 收集的耦合点，由标准 Trace/Event 机制对外暴露。

## 4. 元协议迭代建议
- 后续对于涉及多系统基础设施集成的模块，从第一天起应明确定义“宿主容器观测”与“内核业务观测”的正交边界，避免由于初期便捷性引入逆向依赖。
