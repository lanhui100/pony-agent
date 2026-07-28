## ADDED Requirements

### Requirement: Process execution SHALL have an explicit lifecycle

Pony Agent SHALL 将可持续命令执行表达为 start、poll、write-stdin 与 kill 生命周期，而不是只提供一次性阻塞 shell 调用。

#### Scenario: A process continues after the first call
- **WHEN** 命令在初始等待窗口后仍运行
- **THEN** runtime SHALL 返回稳定 process id 与 running 状态
- **AND** 后续调用 SHALL 能轮询、写入 stdin 或终止该进程

### Requirement: Process output SHALL be drained and bounded

Pony Agent SHALL 在进程运行期间并发排空 stdout/stderr，并使用稳定输出预算。

#### Scenario: A process emits output larger than pipe capacity
- **WHEN** 子进程持续产生大量 stdout 或 stderr
- **THEN** 工具 SHALL NOT 因未读取管道而死锁
- **AND** 结果 SHALL 标明截断状态与丢弃字节数

### Requirement: Process containment SHALL be accurate and platform-aware

Pony Agent SHALL 在 timeout、cancel、explicit kill 和 session shutdown 时清理完整进程树。

#### Scenario: A contained process starts descendants and times out
- **WHEN** 父 shell 与子进程在 deadline 到达时仍运行
- **THEN** platform containment backend SHALL 终止其受管 descendants
- **AND** SHALL 记录 containment backend 与证明结果

#### Scenario: A platform only has a process group
- **WHEN** 平台无法提供防 breakaway 的 containment
- **THEN** runtime SHALL 将 process group 标为 best-effort
- **AND** SHALL NOT 宣称能终止主动 `setsid` 或 double-fork 逃逸进程

### Requirement: Process actions SHALL remain permission-aware

Pony Agent SHALL 对 start、stdin 和 kill 分别执行权限决策并保留审计证据。

#### Scenario: A command requires approval
- **WHEN** process start 的风险事实需要宿主审批
- **THEN** runtime SHALL 在 spawn 前返回 approval_required 或 waiting_host
- **AND** SHALL NOT 先执行再补记录

### Requirement: Autonomous Run SHALL require a real sandbox

Pony Agent SHALL 将 sandbox 与 process containment 分为独立 contract；无人值守 Run 必须在真实 sandbox backend 可用时才可执行。

#### Scenario: The platform lacks a real sandbox backend
- **WHEN** host 请求无人值守 Run 且没有可用 sandbox
- **THEN** runtime SHALL fail closed 并返回 `sandbox_unavailable`
- **AND** SHALL NOT 退化为普通 shell 或只使用 denylist/process group

#### Scenario: The host explicitly requests unsandboxed execution
- **WHEN** host 提供逐次、绑定最终参数的 unsandboxed approval
- **THEN** runtime MAY 执行高风险 mode
- **AND** result 与 trace SHALL 明示 unsandboxed 风险和 approval evidence

### Requirement: Process handles SHALL be owner-bound and secrets-minimizing

Pony Agent SHALL 使用绑定 session/run/owner 的 opaque process handle，并使用最小子进程环境。

#### Scenario: Another session controls a process handle
- **WHEN** 不同 session/run 对 process handle 发起 poll/stdin/kill
- **THEN** runtime SHALL 拒绝该操作
- **AND** SHALL NOT 暴露 OS PID

#### Scenario: A process is spawned
- **WHEN** backend 构建子进程环境
- **THEN** SHALL NOT 默认继承 provider key、session secret 或 ambient proxy credentials
