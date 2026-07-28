## ADDED Requirements

### Requirement: Run SHALL use a bounded process backend

Pony Agent SHALL 让 `Run` 委托到受控 process lifecycle，并对并发输出、timeout、取消、进程树和结果大小设置边界。

#### Scenario: A command produces large output or descendants
- **WHEN** `Run` 执行产生大量输出或创建子进程的命令
- **THEN** runtime SHALL 持续排空受管输出并限制返回大小
- **AND** timeout/cancel SHALL 清理受管进程树

### Requirement: Search and Glob SHALL use standard matching semantics

Pony Agent SHALL 使用真实 regex、glob 与 ignore 规则，并提供确定性结果和预算证据。

#### Scenario: Search receives regex true
- **WHEN** `Search` 以 regex 模式执行
- **THEN** query SHALL 按标准正则表达式编译和匹配
- **AND** 非法 regex SHALL 返回结构化 invalid_pattern

#### Scenario: A repository scan reaches its budget
- **WHEN** Search 或 Glob 达到文件、字节、命中或时间上限
- **THEN** 结果 SHALL 显式标记 truncated 与原因
- **AND** SHALL NOT 把不完整扫描表述为完整结果

### Requirement: WebFetch SHALL enforce web access safety

Pony Agent SHALL 在 WebFetch 中执行网络范围、redirect、响应体和内容类型策略。

#### Scenario: A fetch violates network or response policy
- **WHEN** 目标、redirect 或响应体违反 WebAccessPolicy
- **THEN** 工具 SHALL 在产生不受控读取前停止
- **AND** SHALL 返回机器可读的安全失败

### Requirement: MCP resource surface SHALL expose list, templates, and read operations

Pony Agent SHALL 提供独立的 MCP resource list、template list 与 read 能力，同时保持只读 provenance。

#### Scenario: A host has MCP resource capabilities
- **WHEN** 当前 registry 包含 MCP resources 或 templates
- **THEN** 模型 SHALL 能列出资源、列出模板并读取具体资源
- **AND** 三种操作 SHALL 保留 source id、resource URI 与只读权限事实

#### Scenario: A resource operation uses real source transport
- **WHEN** 模型请求列出或读取 MCP resource
- **THEN** runtime SHALL 通过 source-bound `McpTransport` 执行对应 MCP operation
- **AND** SHALL NOT 将请求 arguments 回显为 resource content
- **AND** SHALL 对分页、超时、断开、畸形响应、blob/text content 与 source replacement 返回结构化结果

#### Scenario: A resource template is listed
- **WHEN** server 暴露 MCP resource templates
- **THEN** runtime SHALL 使用独立 `ResourceTemplate` 类型表达它们
- **AND** SHALL NOT 将 MCP prompt template 误当 resource template

### Requirement: Image inspection SHALL be a dedicated read capability

Pony Agent SHALL 提供 workspace-scoped `view_image` 或等价图片读取工具，不通过 WebFetch 或文本 Read 猜测图片内容。

#### Scenario: A multimodal model inspects a workspace image
- **WHEN** workspace 内图片格式、大小与 provider modality 均受支持
- **THEN** 工具 SHALL 返回规范化 image artifact
- **AND** trace SHALL 只保存受控引用与元数据，不默认复制无限 base64 内容
