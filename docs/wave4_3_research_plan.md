# 波次 4.3 (B087, B084) 调研与测试矩阵设计

## 1. 调研现状

### B087 (结构化日志迁移)
- **现状**：`crates/pony-agent-core/src` 存在 457 处裸 `eprintln!` 调用。大量存在于 `control_plane` 的锁中毒恢复、`session_commit`、`sqlite`、`jobs` 和 `cli.rs`。
- **重构方案**：
  1. 引入标准统一的日志宏包装层或直接使用 `tracing::{info, warn, error, debug}`。
  2. 保持 JSON 结构化格式（带 trace_id，在支持时），并配置相应过滤器。
  3. 保留 CLI 工具必要的面向用户的 stdout/stderr 输出，仅消除库代码中作为诊断用途的裸 `eprintln!`。

### B084 (ACP 宿主接入)
- **现状**：`crates/pony-agent-core/src/acp` 尚未创建任何源文件。
- **方案设计**：
  1. 实现基于 stdio 的 JSON-RPC 2.0 服务端。
  2. 支持协议方法：`initialize`, `session.new`, `session.load`, `session.cancel`, `prompt`。
  3. 支持从 Agent 到 Client 的反向调用：`request_permission` 及事件通知（如 `session-update`）。
  4. 映射现有的 `HostControlPlane` 和事件系统至 ACP 规范。

## 2. 测试矩阵 (Test Matrix)

| 事项 | 验证等级 | 测试范围 | 验收标准 |
| :--- | :--- | :--- | :--- |
| B087 | L2 | 核心库 `eprintln!` 静态扫描 + tracing 行为测试 | 核心库诊断代码零裸 `eprintln!`；输出满足结构化日志断言 |
| B084 | L3 (含对抗) | ACP 协议初始化、会话生命周期与反向调用 | 协议通信全双工通过；畸形 JSON-RPC 优雅拒绝；断连安全处理 |

## 3. 红相锚定计划
在 `tests/acceptance/wave4_3_test.py` 和 `tests/acceptance/adversarial/acp_protocol_test.py` 写入针对 B087 和 B084 的红相断言并进行 Git 提交锚定。
