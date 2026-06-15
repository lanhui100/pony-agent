# Design: First Wave Tool Surface And Legacy Mapping

## 背景

`PA-045` 已经收口了工具协议与暴露边界，`PA-046 / PA-047` 正在收口 workspace 与权限合同。现在需要把这些母合同落到第一层真正对模型与前端可见的工具面上。

当前真实底座已经存在：

- `workspace_read_file`
- `workspace_read_file_segment`
- `workspace_list_files`
- `workspace_search_text`
- `workspace_batch`
- `workspace_gather_context`

问题在于：

- 它们太像内部原语，不适合作为长期产品级工具名
- 不同底层原语与产品级工具的映射关系还没冻结
- 复合工具与显式 `ToolPlan` 是否直接暴露给模型还没写成正式规则

## 设计目标

1. 定义首批模型可见工具面
2. 明确底层原语到产品工具的映射
3. 明确兼容与迁移策略
4. 明确复合工具与显式 `ToolPlan` 的暴露规则

## 非目标

- 不在本 change 中一次引入 Browser / MCP / Workflow 等扩展工具
- 不重写现有底层执行器
- 不要求所有旧工具名立刻删除

## 首批工具面

建议首批模型可见工具：

- `Plan`
- `Read`
- `Search`
- `List`
- `Edit`
- `Write`
- `Run`
- `Ask`

约束：

- 名字保持短、稳定、英文
- 中文短显示名通过展示元数据提供
- 每个工具必须有清晰功能边界

## 三层命名模型

系统应明确区分：

- `model_visible_name`
- `canonical_tool_name`
- `execution_primitive`

约束：

- `model_visible_name` 是模型默认看到并调用的产品级名称，如 `Read`
- `canonical_tool_name` 是 trace / telemetry / frontend 默认聚合的产品级稳定主键
- `execution_primitive` 是实际承载执行的内部原语名，如 `workspace_read_file`
- 默认情况下，`model_visible_name` 可以与 `canonical_tool_name` 相同
- 当同一产品工具映射多个原语时，聚合仍以 `canonical_tool_name` 为准，而不是以原语名分裂

## 最小映射

- `Read`
  - `workspace_read_file`
  - `workspace_read_file_segment`
- `Search`
  - `workspace_search_text`
- `List`
  - `workspace_list_files`
- `Plan`
  - 允许走显式计划协议，不要求映射到单一 workspace primitive

后续如 `Edit / Write / Run / Ask` 对应的新 primitive 尚未完全实现，也应先冻结产品级工具名与协议边界。

## 工具边界

- `Plan`
  - 用于显式计划表达、计划更新或计划驱动执行编排
  - 允许只产出计划而不直接执行子步骤
- `Read`
  - 用于读取文件全文或片段
- `Search`
  - 用于按模式或关键词搜索 workspace 内容
- `List`
  - 用于列出目录、文件或工作区结构
- `Edit`
  - 用于对已有文件执行定向修改
- `Write`
  - 用于新建或整体覆写内容
- `Run`
  - 用于执行命令、脚本或受控运行步骤
- `Ask`
  - 用于向用户或宿主发起澄清、确认或缺失输入请求
  - 不应用作任意泛化执行入口

## 兼容策略

### 内部原语

- 旧 `workspace_*` 名称保留为内部执行原语

### 兼容别名

- 在迁移期允许旧名称作为兼容别名存在
- 但模型默认可见面应优先使用首批产品级工具名
- 兼容别名不应成为长期默认模型可见名
- 是否允许模型直接调用兼容别名，必须由宿主或迁移策略显式决定

### Trace / Telemetry

- 聚合默认按 canonical product tool name
- 必要时保留 execution primitive 以便调试
- 若某次调用通过旧别名触发，前端主展示仍应优先显示产品级工具名，旧原语名仅作次级调试信息

### 生命周期

- 兼容别名只应服务迁移期
- 一旦模型与前端均完成产品级工具名切换，旧别名应进一步降级为纯内部原语
- 兼容层退出条件应至少包括：
  - 默认模型可见面不再直接暴露旧名
  - trace / frontend 已能稳定聚合到 `canonical_tool_name`
  - 关键工作流不再依赖旧名作为外部协议

## 复合工具与 ToolPlan

规则：

- 复合工具可以作为模型可见工具存在
- 其子步骤应通过 `ToolPlan` 与统一 `ToolResult.child_results` 暴露
- `workspace_batch / workspace_gather_context` 这类现有复合原语不应直接成为长期产品级工具名
- 复合工具的权限事实应复用 `PA-047` 的 permission envelope，而不是在本卡重新定义

### Ask

- `Ask` 面向用户或宿主交互
- 若某次 `Ask` 依赖宿主中介，应复用 `PA-047` 的 `host_mediated` 与审批语义
- `Ask` 的结果应是结构化澄清、确认或补充输入，而不是伪装成普通文件/执行结果

### Plan

- `Plan` 是首批固定模型可见工具之一，同时也承接显式计划协议
- `Plan` 可以只产出计划结果，不要求同次调用必须执行
- 若 `Plan` 驱动后续子步骤执行，执行细节应通过统一 `ToolPlan` / `child_results` 暴露

## 验证策略

重点验证：

- 首批工具面是否覆盖基础 agent 最小闭环
- 旧 `workspace_*` 是否被明确降级
- 复合工具与显式 `ToolPlan` 是否有稳定产品层规则
- 前端与 trace 是否能围绕新工具面展示
- 8 个首批工具是否都已具备正式边界，而不是只在文档清单中出现
- 兼容别名是否具备进入、展示与退出规则

## 与后续任务的关系

- `PA-049` 复用本 change 的首批工具面与中文短显示名进入前端与观测展示
