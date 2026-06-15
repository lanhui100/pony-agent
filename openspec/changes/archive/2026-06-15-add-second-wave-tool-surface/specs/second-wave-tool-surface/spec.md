# second-wave-tool-surface Spec

## ADDED Requirements

### Requirement: The system SHALL distinguish first-wave contract gaps from second-wave new capabilities

Pony Agent SHALL 在第二批工具规划中显式区分两类工作：

1. 已经进入首批产品级工具合同、但尚未真实实现的缺口
2. 真正新增的第二批基础工具能力

#### Scenario: A planning document mixes frozen tools with new candidates

- **WHEN** 系统定义下一轮工具实现路线
- **THEN** 它 SHALL 明确指出哪些工具已经在首批合同中被冻结
- **AND** SHALL 明确指出哪些工具属于新增第二批能力

#### Scenario: Edit and Write are already contract-level tools

- **WHEN** 系统规划 `Edit` 与 `Write`
- **THEN** 它 SHALL 将它们视为“首批合同缺口待补齐”
- **AND** SHALL NOT 把它们写成全新第三方扩展能力

### Requirement: The system SHALL prioritize closure of the minimum coding-agent loop

Pony Agent SHALL 先补齐最小 coding agent 闭环，再扩展更外围的探索、外部读取与桥接能力。

#### Phase

`Phase A`

#### Scenario: The system chooses the first implementation batch

- **WHEN** 系统决定第二批工具的首个实现批次
- **THEN** 首批优先范围 SHALL 包括：
  - `Edit`
  - `Write`
  - `RunShell`

#### Scenario: Local execution and editing outrank web and bridge tools

- **WHEN** 系统比较本地编辑执行能力与外部读取能力
- **THEN** `Edit / Write / RunShell` SHALL 优先于 `WebFetch / WebSearch / MCP Resource / ToolSearch`

### Requirement: Edit SHALL have a stable incremental-modification boundary

Pony Agent SHALL 将 `Edit` 定义为“定向修改已有内容”的稳定工具边界。

#### Phase

`Phase A`

#### Scenario: The model wants to update an existing file

- **WHEN** `Edit` 被用于修改文件
- **THEN** 它 SHALL 面向已有文件内容做定向修改
- **AND** SHALL NOT 以“新建文件”为主职责

#### Scenario: Edit is implemented using a structured patch primitive

- **WHEN** `Edit` 底层使用 patch、search-replace 或 range replace 等 primitive
- **THEN** 产品级边界 SHALL 仍保持为“定向修改已有内容”

#### Scenario: Edit cannot find the intended target

- **WHEN** `Edit` 无法找到目标内容
- **THEN** 它 SHALL 返回结构化错误
- **AND** SHALL NOT 静默跳过

#### Scenario: Edit matches multiple ambiguous targets

- **WHEN** `Edit` 匹配到多个候选位置且无法唯一定位
- **THEN** 它 SHALL 返回歧义错误
- **AND** SHALL NOT 默认选择第一个命中

#### Scenario: Edit receives an invalid patch or malformed edit instruction

- **WHEN** `Edit` 输入格式非法
- **THEN** 它 SHALL 返回结构化错误
- **AND** SHALL NOT 静默忽略错误输入

### Requirement: Write SHALL have a stable create-or-overwrite boundary

Pony Agent SHALL 将 `Write` 定义为“新建或整体覆写”的稳定工具边界。

#### Phase

`Phase A`

#### Scenario: The model creates a new file

- **WHEN** `Write` 被用于生成新文件
- **THEN** 它 SHALL 支持文件创建

#### Scenario: Write replaces full file content

- **WHEN** `Write` 被用于整体替换内容
- **THEN** 它 SHALL 允许整文件覆写
- **AND** SHALL 保持与 `Edit` 的职责分离

### Requirement: Run SHALL evolve toward a controlled shell execution boundary

Pony Agent SHALL 把 `Run` 的真实实现方向收口为受控执行边界，而不是长期停留在极轻量 placeholder primitive。

#### Phase

`Phase A`

#### Scenario: The system upgrades Run beyond placeholder behavior

- **WHEN** 系统实现第二批工具能力
- **THEN** `Run` 的近线实现方向 SHALL 包括受控 shell / command 执行能力
- **AND** 产品层 canonical tool name SHALL 继续保持为 `Run`
- **AND** `RunShell` MAY 作为内部实现能力名存在，但 SHALL NOT 作为新的默认对外工具名
- **AND** 该能力 SHALL 至少具有：
  - `cwd`
  - `timeout`
  - `exit_code`
  - `stdout`
  - `stderr`

#### Scenario: Run remains permission-aware

- **WHEN** `Run` 执行真实命令
- **THEN** 它 SHALL 复用既有工具权限与审批合同
- **AND** SHALL NOT 被定义成任意宿主控制入口

#### Scenario: Run encounters a high-risk command

- **WHEN** `Run` 请求执行超出 workspace 边界的系统级高风险命令
- **THEN** 它 SHALL 默认拒绝或进入更高等级审批
- **AND** SHALL NOT 在默认近线范围内直接放行

### Requirement: The system SHALL define a second-wave exploration layer

Pony Agent SHALL 为第二批工具面定义专门的代码库探索增强层。

#### Phase

`Phase B`

#### Scenario: The system plans file-path discovery

- **WHEN** 系统补充文件发现能力
- **THEN** 它 SHALL 将 `Glob` 视为路径模式探索能力
- **AND** SHALL 让其区别于普通 `List`

#### Scenario: Glob differs from List

- **WHEN** 系统同时存在 `Glob` 与 `List`
- **THEN** `List` SHALL 主要承担目录列举职责
- **AND** `Glob` SHALL 主要承担递归路径模式匹配职责

#### Scenario: The system plans content-pattern search

- **WHEN** 系统补充文本模式检索能力
- **THEN** 它 SHALL 将 `Grep` 或等价增强检索 primitive 视为文本模式检索能力
- **AND** SHALL 让其区别于泛化 `Search`

#### Scenario: Product-level Search remains stable while Grep evolves underneath

- **WHEN** 系统增强文本模式检索实现
- **THEN** 产品层 MAY 继续保留 `Search` 作为唯一对外文本检索入口
- **AND** `Grep` MAY 作为实现层 primitive 存在
- **AND** SHALL NOT 因实现增强而强制新增独立对外 canonical tool name

### Requirement: The system SHALL define an external-reading layer separately from local tools

Pony Agent SHALL 将外部网页读取与外部搜索作为独立层定义，而不是混入本地工具边界。

#### Phase

`Phase C`

#### Scenario: The system adds URL reading

- **WHEN** 系统规划 `WebFetch`
- **THEN** 它 SHALL 将其定义为“读取指定 URL 内容”的能力

#### Scenario: The system adds external search

- **WHEN** 系统规划 `WebSearch`
- **THEN** 它 SHALL 将其定义为“外部搜索”能力
- **AND** SHALL 与 `WebFetch` 保持分层

### Requirement: The system SHALL define a bridge-and-governance layer for later phases

Pony Agent SHALL 把 MCP 资源读取与工具发现定义为后续桥接与治理层，而不是与最小 coding loop 同优先级混排。

#### Phase

`Phase D`

#### Scenario: The system plans MCP resource access

- **WHEN** 系统规划第二批桥接能力
- **THEN** 它 SHALL 至少考虑只读 `MCP Resource` 能力

#### Scenario: The system plans deferred tool discovery

- **WHEN** 系统规划大工具池治理
- **THEN** 它 SHALL 将 `ToolSearch` 视为 deferred / dynamic tool discovery 能力
- **AND** SHALL NOT 将其与普通内容搜索混为一谈
- **AND** 在当前 change 中 SHALL 只要求定义其进入条件与后置理由
- **AND** SHALL NOT 要求在当前近线范围内展开实现级设计

### Requirement: The system SHALL define what remains out of scope for this change

Pony Agent SHALL 在第二批工具规划中明确列出当前不进入近线范围的扩展能力。

#### Scenario: The change considers larger product surfaces

- **WHEN** 系统评估 Browser、Workflow、Thread、Automation、LSP 等更大能力面
- **THEN** 本 change SHALL 明确它们不属于当前第二批基础内置工具面的实现范围
