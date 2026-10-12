# 0029 工作区外部访问提权机制（基于 DSH 的升权引导与受控授权协议）

Status: implemented

## 候选方案

### 方案 A：全局粗粒度开关 / 一键关闭沙箱
- **描述**：在设置或提示词中提供一个全局“允许读写全盘”的开关，打开后彻底禁用所有路径越界检查。
- **落选原因**：破坏了 Fail-Closed 安全边界底线。一旦失控，恶意/有缺陷的 Agent 指令可能污染或删除系统关键目录，缺乏最小特权（Least Privilege）和单次审查审计轨迹。

### 方案 B：基于 DSH 模式的升权引导与单次/信任路径受控授权（选定）
- **描述**：
  1. 越权拦截时下发标准化提示：`outside_workspace_write_denied: [sandbox: escalation available — 使用 request_permission 请求授权]`；
  2. 扩展底层 `AuthorizeStore`，支持受控的 `read` 与 `read-write` 作用域；
  3. 通过 `PendingControlRequest`（Kind: `approval`）生成人机确认请求，前端弹窗供用户审查目标路径、操作类型及 Agent 的理由（`justification`）；
  4. 用户批准后注入授权白名单，Agent 即可重试通过。
- **选定原因**：参考成熟的 DeepSeek Harness (DSH) 提权机制，操作透明、有据可查、具备人机交互（HITL）闭环与最小暴露面。

## 背景
Pony Agent 对工作区外部访问采取严格的物理防御：
- 外部读操作在未授权时返回 `requires_authorization`；
- 外部写操作一律物理阻断返回 `outside_workspace_write_denied`，且 `authorize_path` 接口拒绝任何写 scope。
这导致 Agent 在遇到需要跨项目读写、导出成果物或修改外部配置时被彻底死锁，且前端缺乏直观的提权审核交互。

## 决策
1. **统一升权提示与错误契约**：
   - 越权写被阻断时，除返回错误码外，追加标准 DSH 格式的自愈标记：
     `[sandbox: escalation available — retry with explicit authorization]`。
2. **AuthorizeStore 扩展写授权支持**：
   - 允许 `authorize_path` 支持 `scope: "read-write"`（或 `"write"`），使得用户显式批准的外部路径能够被合法写入。
   - 保证路径规范化（`normalize_canonical`）与词法逃逸防御（`..` 拦截）依然严格生效。
3. **前端 UI/UX 提权卡片交互**：
   - 前端新增 `PermissionElevationCard` 交互组件，承载外部路径读写授权请求。
   - 用户可选择：拒绝（Reject）、单次/永久信任授权该路径。
   - 授权成功后自动调用 `authorize_path` 写入底层授权表，解除阻断。

## 非目标 (Non-Goals)
1. 默认开放全盘写权限。
2. 移除 Windows Job Object 或受限沙箱底层进程隔离。
