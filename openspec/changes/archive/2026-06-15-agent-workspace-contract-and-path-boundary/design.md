# Design: Agent Workspace Contract And Path Boundary

## 背景

当前 `AgentRuntimeBuilder.workspace_root(...)` 与 `ToolRouter::with_workspace_root(...)` 已经提供了 workspace 注入路径，说明工作区边界已经是 runtime 的现实依赖，而不是未来可选能力。

同时，`tools.rs` 已经具备：

- 相对路径解析到 workspace root
- 绝对路径 canonicalize 后的越界检查
- workspace-relative 展示
- 缺扩展名或路径不精确时的有限修复

现阶段缺失的不是能力，而是正式合同：

- 什么叫“当前 agent workspace”
- 哪些工具默认受它约束
- 相对路径、绝对路径、展示路径、修复路径之间的关系
- workspace 对 trace、permission scope、host preset 的意义是什么

## 设计目标

1. 定义正式的 workspace 边界合同
2. 统一路径解析、路径展示与越界防护语义
3. 让 workspace 成为 runtime / tool / trace / permission 的共同上下文
4. 为 future host workspace policy 保留稳定 seam

## 非目标

- 不在本 change 中实现多 workspace 产品体验
- 不在本 change 中定义完整审批策略
- 不重写现有工具实现，只要求把已有能力升级为正式合同

## WorkspaceContext

系统应以 `WorkspaceContext` 或等价结构表达当前 agent workspace。

最小字段建议：

- `root`
- `display_name`
- `writable`
- `default_shell_cwd`
- `policy`

约束：

- `root` 是本次 agent run 的正式工作区根目录
- `default_shell_cwd` 是执行类工具的默认工作目录
- `writable` 至少表达当前 workspace 是否允许写入
- `policy` 用于表达 future host workspace policy seam，而不是让宿主在外部复制路径判断逻辑

## 路径语义

### 1. 输入路径

模型或上层调用者默认传入相对路径。

规则：

- 相对路径默认相对 `WorkspaceContext.root` 解析
- 绝对路径仅在 canonicalize 后仍位于 workspace 边界内时允许通过
- 空路径、无意义路径、越界路径必须结构化拒绝

### 2. Canonical path

系统内部执行应基于 canonical path，而不是字符串路径推断。

规则：

- 任何读写、目录枚举或基于路径的执行前都应 canonicalize
- canonicalize 失败应映射为结构化工具错误
- canonicalize 成功但越出 workspace root 时应映射为 `out_of_scope`
- 不允许以字符串前缀判断替代 canonical path containment check

统一顺序：

1. 解析输入路径
2. canonicalize 目标路径
3. 做 workspace containment check
4. 才允许执行实际访问

### 3. Display path

前端、trace、monitor 与工具摘要默认展示 workspace-relative path。

规则：

- 只要目标位于 workspace 内，默认展示相对路径
- 仅在无法安全计算相对路径时，才退回绝对路径或宿主提供的受控显示
- 展示路径是 presentation data，不替代内部 canonical path
- 若绝对路径可能暴露不应直接显示的宿主信息，则应优先走受控显示或宿主层脱敏展示

### 4. Path repair

路径修复可以存在，但必须受限。

规则：

- 只允许在 workspace 边界内进行有限搜索与修复
- 必须有稳定的停止条件与最大搜索边界
- 若修复结果存在多个候选，必须返回结构化冲突，而不是猜测
- 路径修复是便捷能力，不得绕过 workspace 边界或 canonicalize 规则
- 不允许跨同名目录做无约束模糊猜测

## 工具边界

默认受 workspace 约束的工具至少包括：

- `Read`
- `Search`
- `List`
- `Edit`
- `Write`

执行类工具如 `Run`：

- 默认工作目录来自 `WorkspaceContext.default_shell_cwd`
- 若允许显式 cwd，则该 cwd 仍必须受 workspace 合同约束
- 跨出 workspace 执行不能被本 change 默认放行；如后续允许，必须由权限合同显式定义

## Workspace 作为共同上下文

workspace 不只是路径容器，还应成为这些层的共同上下文：

- runtime：本次 turn 的正式工作区边界
- tool：文件/目录/执行类工具的默认范围
- trace：展示相对路径与越界拒绝原因
- permission：后续 `workspace.read / workspace.write / workspace.execute` scope 的基础
- host preset：future Tauri / HTTP-SSE / CLI / service host 注入不同 workspace policy 的 seam

## Host seam

宿主可以决定 workspace 如何被选择或注入，但不得复制工具层路径语义。

约束：

- workspace 选择可以是桌面当前项目目录、CLI cwd、服务端会话工作区等
- 一旦进入 runtime，路径解析、越界规则与展示规则必须由统一 workspace 合同处理
- host 不得绕过 workspace 合同自行决定哪些路径“看起来可读写”

## 验证策略

本 change 的验证重点：

- workspace root 是否成为正式运行时输入
- `WorkspaceContext` 是否足够提供共同上下文字段
- 相对路径、绝对路径、display path 与 canonical path 是否被清晰区分
- 越界访问是否有稳定拒绝语义
- path repair 是否受 workspace 边界与停止条件约束
- 执行类工具是否有明确默认工作目录与跨 workspace 默认拒绝规则
- future host 是否可注入 workspace policy，而不复制工具逻辑

## 与后续任务的关系

- `PA-047` 复用本 change 定义的 workspace scope 进入权限合同
- `PA-048` 在本 change 基础上给首批文件类与执行类工具定义清晰路径行为
- `PA-049` 复用本 change 的 display path 规则进入前端与 trace 展示
