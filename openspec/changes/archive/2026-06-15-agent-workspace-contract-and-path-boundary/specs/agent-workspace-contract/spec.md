# agent-workspace-contract Spec

## ADDED Requirements

### Requirement: The system SHALL define workspace as a formal runtime boundary

Pony Agent SHALL 将 workspace 视为 agent run 的正式上下文边界，而不是隐式实现细节。

#### Scenario: Runtime is created for a host

- **WHEN** 某个宿主创建一次 agent runtime 或 turn 执行上下文
- **THEN** 系统 SHALL 提供显式 workspace 边界输入
- **AND** SHALL NOT 仅依赖隐式 `current_dir` 作为唯一工作区真相源

#### Scenario: Workspace context is shared across layers

- **WHEN** runtime、tool、trace、permission 或 host seam 读取当前工作区上下文
- **THEN** 系统 SHALL 能提供统一 `WorkspaceContext` 或等价正式结构
- **AND** 该结构至少 SHALL 表达 workspace root 与执行类工具默认工作目录

### Requirement: Relative paths SHALL resolve against the workspace root

Pony Agent SHALL 将相对路径统一解释为当前 workspace root 下的路径，而不是依赖调用方各自决定解析基准。

#### Scenario: A file tool receives a relative path

- **WHEN** 某个文件或目录类工具接收到相对路径
- **THEN** 系统 SHALL 将其相对于当前 workspace root 解析

### Requirement: Workspace access SHALL canonicalize before path-based access

Pony Agent SHALL 在所有基于路径的访问前完成 canonicalize 与 containment check，以保证 workspace 边界判定稳定一致。

#### Scenario: A path-based tool is about to access a target

- **WHEN** 某个读写、目录枚举或基于路径的执行即将访问目标
- **THEN** 系统 SHALL 先完成路径解析
- **AND** SHALL canonicalize 目标路径
- **AND** SHALL 在 canonical path 上执行 workspace containment check
- **AND** 仅在 containment check 通过后才 MAY 继续访问

#### Scenario: An absolute path is provided

- **WHEN** 某个工具接收到绝对路径
- **THEN** 系统 SHALL 先 canonicalize 该路径
- **AND** 只有当 canonical path 仍位于 workspace 边界内时，系统才 MAY 继续执行

#### Scenario: A canonical path escapes the workspace

- **WHEN** 某个路径 canonicalize 后越出 workspace root
- **THEN** 系统 SHALL 拒绝该访问
- **AND** SHALL 以结构化越界失败语义返回，而不是继续猜测执行

### Requirement: Display paths SHALL prefer workspace-relative representation

Pony Agent SHALL 默认以 workspace-relative 形式展示工作区内路径，并把绝对路径回退限制在受控展示场景。

#### Scenario: A path is shown in tool output or trace

- **WHEN** 前端、trace、monitor 或工具摘要展示某个工作区内路径
- **THEN** 系统 SHALL 优先展示 workspace-relative path

#### Scenario: Relative display cannot be computed safely

- **WHEN** 系统无法安全计算 workspace-relative path
- **THEN** 系统 MAY 退回受控绝对路径显示或宿主受控展示
- **AND** SHALL NOT 因为展示回退而改变内部 canonical path 真相

### Requirement: Path repair SHALL remain bounded by workspace rules

Pony Agent SHALL 允许有限路径修复，但该能力必须从属于 workspace 边界、canonicalize 规则与稳定停止条件。

#### Scenario: A user or model provides an imprecise path

- **WHEN** 系统尝试修复缺扩展名或不精确路径
- **THEN** 修复过程 SHALL 只在 workspace 边界内进行
- **AND** SHALL NOT 绕过 canonicalize 与越界检查

#### Scenario: Path repair yields multiple candidates

- **WHEN** 路径修复得到多个候选
- **THEN** 系统 SHALL 返回结构化冲突信息
- **AND** SHALL NOT 擅自猜测其中一个候选继续执行

#### Scenario: Path repair search is bounded

- **WHEN** 系统执行路径修复搜索
- **THEN** 该搜索 SHALL 有稳定边界与停止条件
- **AND** SHALL NOT 变成无约束递归扫描

### Requirement: File and directory tools SHALL default to workspace scope

Pony Agent SHALL 让文件与目录类工具默认工作在当前 workspace 范围内，而不是让每个工具单独定义根范围。

#### Scenario: A read/search/list/edit/write tool runs

- **WHEN** 某个文件或目录类工具执行
- **THEN** 它 SHALL 默认只在当前 workspace 边界内工作

### Requirement: Execute tools SHALL have a workspace-default execution boundary

Pony Agent SHALL 为执行类工具提供明确的 workspace 默认执行目录，并将跨 workspace 执行视为需后续合同显式放行的例外。

#### Scenario: A Run-like tool executes without an explicit cwd

- **WHEN** 某个执行类工具运行且未显式指定工作目录
- **THEN** 它 SHALL 默认使用 `WorkspaceContext.default_shell_cwd`

#### Scenario: A Run-like tool attempts to execute outside workspace

- **WHEN** 某个执行类工具尝试在 workspace 边界外执行
- **THEN** 本 change SHALL NOT 默认允许该行为
- **AND** 若未来允许，必须由后续权限合同显式定义

### Requirement: Workspace SHALL become shared context across layers

Pony Agent SHALL 让 workspace 成为 runtime、tool、trace、permission 与 host seam 共同复用的正式上下文真相源。

#### Scenario: A tool failure is caused by path scope

- **WHEN** 某次工具调用因 workspace 边界失败
- **THEN** runtime、trace 与后续权限合同 SHALL 能引用同一份 workspace scope 语义

#### Scenario: A host injects a workspace policy

- **WHEN** 不同宿主为 runtime 注入不同工作区策略
- **THEN** 宿主 MAY 决定 workspace 如何选择
- **AND** 一旦进入 runtime，路径解析、越界规则与展示规则 SHALL 复用统一 workspace 合同
