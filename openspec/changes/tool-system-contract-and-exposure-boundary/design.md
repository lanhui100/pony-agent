# Design: Tool System Contract And Exposure Boundary

## 背景

当前 Pony Agent 的工具能力已经不是“空白起点”：

- `ToolRouter` 已提供本地工具执行与 workspace 路径边界
- `ToolCall.plan` 与 `ToolPlan` 已把复合工具和显式计划提升为一等字段
- capability bridge 已经把 builtin / MCP / skill 相关能力纳入统一 capability 读面与执行入口
- telemetry / control-plane / session trace 已经能够记录工具活动

现阶段真正缺失的是一份稳定的工具系统合同，用来回答这些问题：

- 模型应该看到哪些工具名，哪些名字只是内部原语
- 哪个标识用于模型协议、哪个标识用于跨来源聚合、哪个标识用于运行时执行解析
- 工具定义至少要带哪些字段
- 工具调用、工具结果和工具失败如何结构化表达
- 不同来源的工具如何共享同一套结果与失败语义
- 前端中文短显示名、trace 展示字段与运行时工具标识之间如何分层

## 设计目标

1. 为 Pony Agent 定义统一工具系统协议
2. 明确模型可见工具名与内部执行原语名分层
3. 明确三层标识模型
4. 统一工具结果与失败语义
5. 统一工具分类与暴露策略
6. 让 capability / skill / builtin / composite tool 复用同一套合同
7. 为后续 workspace、权限、首批工具面与前端呈现任务提供母边界

## 非目标

- 不在本 change 中直接实现所有首批工具
- 不直接完成 workspace 路径与审批 UI 细节
- 不重写 capability bridge、skills registry 或前端信息架构
- 不要求本轮把所有旧工具名立刻迁移下线

## 核心分层

### 1. 模型可见工具层

这层服务于：

- model tool selection
- prompt/tool schema stability
- 前端主要工具活动展示

要求：

- 工具名简洁、短、稳定
- 工具名不泄漏过多内部实现细节
- 工具显示元数据（如中文短名）与底层标识分离

推荐形态：

- `Plan`
- `Read`
- `Search`
- `List`
- `Edit`
- `Write`
- `Run`
- `Ask`

本 change 不冻结这份首批清单，但要求这类工具名属于产品级协议层，而不是 `workspace_*` 这一类内部实现名。

命名约束：

- 产品级模型工具名优先使用单词动词或高频动作词
- 工具名应避免暴露 `workspace_`、`builtin_`、`mcp_` 等实现前缀
- 当前建议统一使用 PascalCase
- `Read / List / Search` 等近义工具必须定义清晰边界，不允许长期重叠同义暴露

### 2. 内部执行原语层

这层服务于：

- 本地执行器
- 兼容现有 `workspace_*` 能力
- 复合工具内部展开
- capability-backed resolution 之后的实际调用

要求：

- 可继续保留当前底层工具粒度
- 可作为 composite tool 的子步骤
- 不要求直接暴露给模型

例如：

- `workspace_read_file`
- `workspace_read_file_segment`
- `workspace_list_files`
- `workspace_search_text`
- `workspace_batch`
- `workspace_gather_context`

### 3. 工具协议层

这层是本 change 的核心，要求统一：

- `ToolDefinition`
- `ToolCall`
- `ToolResult`
- `ToolFailureKind`
- `ToolExposure`
- `ToolKind`

### 4. 三层标识模型

为避免模型协议、聚合口径和底层原语混淆，系统要求区分三层标识：

1. `name`
   模型可见工具名。用于模型工具协议与默认运行时入口。
2. `canonical_tool_name`
   跨来源聚合的逻辑工具标识。用于 trace、telemetry、monitor 与 session drilldown 聚合。
3. `execution_primitive`
   运行时最终解析到的底层执行原语名。用于 builtin primitive、capability-backed action 或 composite tool 子步骤的实际执行。

约束：

- `name` 与 `canonical_tool_name` MAY 相同，但不要求相同
- `execution_primitive` SHALL NOT 取代 `name` 或 `canonical_tool_name`
- trace/telemetry 聚合默认按 `canonical_tool_name` 进行
- 一个模型可见工具 MAY 对应多个 `execution_primitive`

最小示例：

- `name = "Read"`
- `canonical_tool_name = "read"`
- `execution_primitive = "workspace_read_file"` 或 `workspace_read_file_segment`

## 合同设计

### ToolDefinition

`ToolDefinition` 至少应覆盖：

- `name`
- `canonical_tool_name`
- `execution_primitive`
- `display_name_zh`
- `description`
- `kind`
- `exposure`
- `input_schema`
- `output_schema`
- `policy_metadata`

约束：

- `name` 是稳定英文标识，用于模型与 runtime 协议
- `canonical_tool_name` 是跨来源聚合的逻辑工具标识
- `execution_primitive` 是运行时最终解析到的内部执行原语名
- `display_name_zh` 属于展示元数据，不得作为底层唯一标识
- `kind` 与 `exposure` 必须结构化，而不是靠名字推断
- `policy_metadata` 可以为后续审批/权限合同预留字段，但其正式语义由 `PA-047` 定义

### ToolKind

系统至少支持以下类别：

- `read`
- `search`
- `write`
- `execute`
- `plan`
- `interactive`
- `composite`
- `external`

约束：

- `builtin / capability / skill` 是来源视角，不替代 `ToolKind`
- `ToolKind` 用于权限、planner、trace 与前端图标/语义聚合

### ToolExposure

系统至少支持：

- `model_visible`
- `internal`
- `deferred`

含义：

- `model_visible`：默认进入模型可见工具表
- `internal`：只供 runtime/composite/capability 内部使用
- `deferred`：已注册但不默认暴露，需通过 discover/search 等路径显式进入当前 turn 能见面

`deferred` 的最小生命周期规则：

- 只允许由受控 discover/search/host mediation 路径提升为当前 turn 可见工具
- 默认作用域为“当前 turn”
- 若宿主或后续合同允许跨 turn/session 保留，必须显式记录扩展来源与保留范围
- trace SHALL 能记录某工具是“原本 deferred，后被提升为 visible”

### ToolCall

`ToolCall` 至少应覆盖：

- `name`
- `canonical_tool_name`
- `execution_primitive`
- `arguments`
- `plan`
- `display_name_zh`
- `kind`
- `exposure`

约束：

- `plan` 保持一等字段，不能回退为隐式 JSON 约定
- composite tool 的显式计划必须能通过统一 `ToolCall` 被 trace/telemetry 消费

### ToolResult

`ToolResult` 至少应覆盖：

- `status`
- `summary`
- `data`
- `child_results`
- `error`
- `artifacts`
- `duration_ms`
- `canonical_tool_name`
- `display_name_zh`
- `kind`

约束：

- `summary` 是面向模型和前端的简明结果摘要
- `data` 是结构化机器可消费结果
- `child_results` 用于复合工具或显式计划型工具的子步骤结果
- `error` 是结构化失败对象，不允许只留自由文本
- `artifacts` 只用于路径、文件片段、渲染对象、引用对象等外显附件或引用物

这三类槽位不得混用：

- `data`：主结果数据
- `child_results`：子步骤结果
- `artifacts`：附件或引用对象

### ToolFailureKind

系统至少支持以下失败种类：

- `invalid_input`
- `not_found`
- `permission_denied`
- `approval_required`
- `out_of_scope`
- `execution_failed`
- `timeout`
- `source_unavailable`
- `conflict`
- `unsupported`

约束：

- capability / skill / builtin / composite tool 都必须映射回这一套失败种类
- 自由文本只作为补充说明，不得替代结构化失败 kind

### ToolError

`ToolResult.error` 至少应覆盖：

- `kind`
- `message`
- `details?`
- `retryable?`
- `source?`

约束：

- `kind` 必须来自统一 `ToolFailureKind`
- `message` 面向前端与调试读面，可读但不代替结构化 kind
- `details` 用于补充路径、schema、provider、approval context 等结构化上下文
- `retryable` 和 `source` 可选，但若系统已知该信息则应保真保留

## 来源统一

### builtin tools

- 保留当前本地执行能力
- 按统一 `ToolDefinition / ToolResult / ToolFailureKind` 对外暴露

### capability-backed tools

- resolve 仍发生在 capability bridge 内
- 但最终呈现的工具定义、结果和失败语义必须回到统一工具合同
- `policy_metadata` 中出现的审批与权限语义，在本 change 中只作为占位元数据；其正式含义由 `PA-047` 收口

### skill-composed tools

- skill 不应重新发明第三套工具协议
- tool-only skill 的执行结果需要映射回统一工具结果合同

### composite tools

- composite tool 可以拥有自己的模型可见工具名
- 子步骤继续使用内部原语或 capability resolve
- 子步骤结果通过统一 `ToolPlan` 与 `ToolResult.child_results` 暴露

最小映射示例：

- `Read`
  - `canonical_tool_name = "read"`
  - `execution_primitive = "workspace_read_file"` 或 `workspace_read_file_segment`
- `Search`
  - `canonical_tool_name = "search"`
  - `execution_primitive = "workspace_search_text"`
- `List`
  - `canonical_tool_name = "list"`
  - `execution_primitive = "workspace_list_files"`

## 展示层元数据

中文短显示名属于展示层元数据：

- 用于前端工具活动展示
- 用于 trace / monitor / session drilldown 简洁标签
- 不作为底层唯一工具标识

这保证：

- 底层协议保持英文稳定
- UI 可以按中文短名做本地化
- 后续多语言或不同宿主不会污染 runtime 协议

## 验证策略

本 change 的验证重点不是具体工具实现，而是合同稳定性：

- 工具定义是否足够覆盖 builtin/capability/skill/composite 四类来源
- 三层标识模型是否足够支持模型协议、聚合口径与底层执行分层
- 结果结构是否足够支持模型、trace、前端与后续权限合同
- 暴露策略是否能支持 model-visible / internal / deferred 三层
- 中文短显示名是否被明确定义为展示元数据
- 旧 `workspace_*` 名称是否被明确降级为内部原语，而不是继续作为长期产品协议

## 与后续任务的关系

- `PA-046` 在本合同上定义 workspace 边界与路径语义
- `PA-047` 在本合同上收口权限事实与审批语义
- `PA-048` 在本合同上定义首批工具面与旧工具映射
- `PA-049` 在本合同上收口观测与前端呈现读面
