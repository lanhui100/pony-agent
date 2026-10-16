# wave-19 契约冻结矩阵：Fail-Closed 策略评估器恢复与宿主审批控制面闭环

## 1. 背景与缺陷诊断
- 现状缺陷：`governed_executor.rs` 强制注入 `LegacyCompatiblePolicyEvaluator`，对 Write/Execute 等高危操作一律返回 `Allow`。
- 闭环缺失：`GovernedDispatcher::approve_control_request` 虽已实现完备的 CAS 校验，但 `HostControlPlane` 和 Tauri 宿主层完全未暴露该命令，导致前端/外部宿主即使收到挂起事件也无法批准。

## 2. 目标契约 (C1 ~ C4)

### C1: 默认 Fail-Closed 策略
- `build_governed_executor` 默认使用 `DescriptorPolicyEvaluator`。
- 凡声明 `ToolPermissionScope::WorkspaceWrite` 或 `ToolPermissionScope::WorkspaceExecute` 的工具，在未经预授权时，调度器必须返回 `PermissionVerdict::ApprovalRequired` 并持久化 `PendingControlRequest`。

### C2: 宿主控制面接入 `approve_control_request`
- `HostControlPlane` 暴露 `approve_control_request(request_id: &str, expected_version: u64, authorization: Option<Value>) -> Result<Value, String>`。
- 内部调用 `dispatcher.approve_control_request` 完成 CAS 乐观锁消费。

### C3: Tauri 宿主桥接命令
- `src-tauri/src/lib.rs` 增加 `#[tauri::command] fn control_approve_request(...)`。

### C4: 兼容性与直通豁免（测试/开发模式可控注入）
- 保持 `build_governed_executor_with_policy_evaluator` 或支持通过环境/配置显式指定 `LegacyCompatiblePolicyEvaluator`，确保既有不需要审批交互的基准/回归套件可平滑运行。
