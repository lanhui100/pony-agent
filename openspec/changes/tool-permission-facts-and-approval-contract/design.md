# Design: Tool Permission Facts And Approval Contract

## 背景

工具权限现在已经不是“以后再做”的话题：

- capability bridge 已经对 capability / skill 来源保留审批与权限事实
- 工具错误模型已经需要区分 `permission_denied / approval_required / out_of_scope`
- `PA-045` 已经把 `policy_metadata` 作为正式占位元数据留下

当前真正缺失的是：

- 权限事实的统一结构
- 工具定义层与权限决策层的边界
- 不同工具来源如何继承、聚合或放大底层权限要求
- 前端、trace、monitor 应该读到哪些稳定权限字段

## 设计目标

1. 定义统一 `ToolPermissionFacts`
2. 定义统一权限失败语义
3. 明确工具定义层、权限决策层与前端读面的边界
4. 让 builtin / capability / skill / composite tool 共享同一套权限口径

## 非目标

- 不在本 change 中实现完整审批 UI
- 不在本 change 中实现所有宿主交互细节
- 不重写 capability bridge 业务主逻辑

## ToolPermissionFacts

建议最小字段：

- `requires_approval`
- `permission_scope`
- `host_mediated`
- `permission_profile`
- `approval_mode`
- `decision_source`

约束：

- `requires_approval` 是稳定布尔事实，表示该工具调用在当前上下文下是否进入审批路径
- `permission_scope` 是稳定机器可读范围，至少可表达 `workspace.read / workspace.write / workspace.execute`
- `host_mediated` 表示该调用是否必须经过宿主中介，而不是 runtime 直接放行
- `permission_profile` 用于高层归纳展示，但不替代底层 scope；若存在冲突，以 `permission_scope` 为准
- `approval_mode` 是稳定模式字段，至少允许区分 `none / required / inherited / delegated`
- `decision_source` 表示最终决策主要来自 `runtime / host / policy / inherited-child`

字段分层建议：

- definition-time facts：
  - `permission_scope`
  - `permission_profile`
  - `approval_mode`
  - `host_mediated`
- decision-time facts：
  - `requires_approval`
  - `decision_source`
- frontend-read stable envelope：
  - 上述字段均应可被 trace / monitor / frontend 读取
  - 前端可以展示摘要，但不得自行补造第二份权限真相

## 分层边界

### 工具定义层

负责声明：

- 该工具可能需要什么权限事实
- 这些事实如何进入 `ToolDefinition` / `ToolCall`
- 稳定 definition-time permission envelope

不负责：

- 最终是否放行
- 用户如何确认
- 宿主如何弹出审批交互

### 权限决策层

负责：

- 判断此次调用是否允许
- 判断是否需要审批
- 在不改写 definition-time facts 的前提下追加 decision-time facts
- 输出结构化拒绝或放行结果

不负责：

- 重新定义工具协议
- 直接篡改工具结果结构

### 前端读面

负责：

- 读取统一 permission envelope
- 展示拒绝/待审批状态
- 展示最终决策结果

不负责：

- 推导新的权限真相

### 跨层交换面

系统至少应存在统一 permission envelope 或等价结构，满足：

- `ToolDefinition` 提供 definition-time facts
- `ToolCall` 或其执行上下文保留当次调用的有效权限事实
- `ToolResult` 或结构化失败结果保留 decision-time outcome
- trace / monitor / frontend 读取同一 permission envelope，而不是各自产生派生结构

## 来源统一

### builtin tools

- 默认由工具定义层提供基础权限事实

### capability-backed tools

- 必须保留底层 capability 的真实审批与 scope 信息
- 不允许在 bridge 层把更强权限压缩成更弱摘要

### skill-composed tools

- 权限必须保守聚合到底层最强要求
- 不允许 skill 用较弱 summary 掩盖底层差异

### composite tools

- 需显示聚合后权限事实
- 同时保留子步骤权限事实可追溯

### 聚合口径

- `permission_scope` 的聚合应至少覆盖所有子步骤所需 scope 的并集
- `requires_approval` 只要任一子步骤需要审批，聚合结果即为 `true`
- `host_mediated` 只要任一子步骤要求宿主中介，聚合结果即为 `true`
- `permission_profile` 可以做高层归纳，但不得把更强底层 scope 降级成更弱摘要
- `decision_source` 若来自聚合，应能表达其源于子步骤继承或宿主决策
- composite/skill 至少应保留每个子步骤的：
  - `step_id` 或等价稳定标识
  - `permission_scope`
  - `requires_approval`
  - `decision_source`

## 失败语义

系统至少支持：

- `permission_denied`
- `approval_required`
- `out_of_scope`

并至少附带：

- `decision_source`
- `scope`
- `details`

区分规则：

- `approval_required`：调用仍可在审批后继续，失败语义重点是“待授权”
- `permission_denied`：调用已被明确拒绝，失败语义重点是“不可执行”
- `out_of_scope`：调用越出合同允许范围，至少应指出越出的 scope 类型，如 workspace scope 或 permission scope
- `out_of_scope`` 不应退化成泛化的 `permission_denied`，其 scope 边界应与 `PA-046` 的 workspace 合同复用同一真相源

## 验证策略

重点验证：

- `ToolPermissionFacts` 是否足够覆盖 builtin/capability/skill/composite 四类来源
- 技能与复合工具是否保守继承最强权限
- 前端与 trace 是否能读取稳定权限字段
- 权限失败是否不再退化为自由文本
- definition-time / decision-time / frontend-read 三层交换面是否使用同一 permission envelope
- `out_of_scope` 是否能区分 workspace scope 与其他 permission scope
- `approval_required` 是否至少带有 `decision_source + scope`

## 与后续任务的关系

- `PA-048` 复用本 change 定义的权限事实进入首批工具面
- `PA-049` 复用本 change 的权限读面进入前端与 monitor 展示
