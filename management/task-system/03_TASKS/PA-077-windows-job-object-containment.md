# PA-077 Windows Job Object 进程树 containment

- Task ID: PA-077
- 标题: Windows Job Object 进程树 containment（PA-076 后续）
- 状态: Review（实现完成；closeout gate blocked）
- Closeout gate: Blocked（工作树归属、门禁复现与跨平台证据尚未闭合）
- 优先级: P1
- 复杂度: C（安全敏感、Windows FFI、跨 ProcessBackend 生命周期边界）
- 负责人: @orchestrator + implementation
- 创建时间: 2026-09-30
- 预计工作量: 2 天（方案 A：spawn 后立即挂入 Job；不含完整 SandboxBackend）
- OpenSpec Change: `openspec/changes/archive/2026-09-30-windows-job-object-containment/`
- Canonical Spec: `openspec/specs/windows-process-containment/spec.md`
- ADR: `docs/decisions/0018-windows-job-object-containment.md`（三路设计审核通过，决定已采纳；工程进度由本卡记录）

## 背景

- **实施前基线（PA-077 启动时）**：`ProcessManager` 只调用 `Child::kill()`，只能保证顶层进程终止，无法可靠终止其已派生子孙。
- **实施前基线（PA-077 启动时）**：`sandbox.rs` 已有 `SandboxSupportMatrix::WindowsJobObject` 与 fail-closed 门禁，但真实 Win32 Job Object 实现尚不存在。

PA-076 评估确认：`windows-sys` 的 Job Object API 可用，最小聚焦实现不需要完整 SandboxBackend。该卡只补进程树 containment，不将 Job Object 宣称为文件、网络或审批沙箱。

## 目标

1. 在 Windows 上为每个受管子进程创建独立 Job Object。
2. 设置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，并明确不允许 breakaway。
3. `ProcessManager::start` 在 spawn 后立即尝试把子进程挂入 Job；Job setup、assignment、membership verification 或 pipe handoff/rollback 任一步失败，必须不发布成功句柄，对 direct child 执行有界终止/回收，并释放 Job 所有权；spawn→assign 窗口内创建的后代可能逃逸，不提供绝对清理保证。
4. `kill`、`shutdown`、timeout、timer 与 last-owner 清理路径 SHALL 通过 Job Object 终止已成功挂入 Job 的成员；终止错误必须可见，并使用 Job close/kill-on-close 作为兜底，随后释放 Job 句柄。
5. 保持 Linux/macOS 编译与现有非 Windows 行为不变。
6. 保持 `NoSandboxBackend` / `enforce_sandbox` 的 fail-closed 审批门禁不变；Job Object 只负责 containment。

## 非目标

- 不实现完整 `SandboxBackend`（文件/workspace、网络、AppContainer、受限令牌）。
- 不把 Windows Job Object 注册成允许无人值守 Run 的 sandbox backend。
- 不在本卡引入裸 `CreateProcessW`、`STARTUPINFOEX` 或 suspended-start 严格消除 spawn-to-assign race；本卡明确接受方案 A 的受调度影响、无固定时间上界的窗口并记录证据。
- 不改变 `ProcessBackend` 的公共序列化协议或前端输出协议。

## 基线证据

- `crates/pony-agent-core/src/agent/process.rs:261-313`：当前 `Command::spawn` 后只保存 `Child`，无 Job 句柄。
- `crates/pony-agent-core/src/agent/process.rs:18-24`：当前文档明确声明不承诺进程树 containment。
- `crates/pony-agent-core/src/agent/sandbox.rs:14-23`：PA-076 已记录真实 backend 为后续工作，当前保持 fail-closed。
- `management/task-system/02_REVIEWS/2026-08-04-pa076-sandbox-backend-evaluation.md:24-57`：Win32 API、方案 A race 与可行性评估。

## 技术方案（待审核）

### Windows Job 封装

新增 Windows-only 私有模块（可放在 `process.rs` 内部或 `process/windows_job.rs`），只管理进程生命周期，不实现 SandboxBackend：

- `CreateJobObjectW` 创建无名 Job。
- `SetInformationJobObject` 设置 `JOBOBJECT_EXTENDED_LIMIT_INFORMATION.BasicLimitInformation.LimitFlags`，只置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，不置 breakaway flags。
- `Child::as_raw_handle` 借用现有子进程句柄，避免按 PID 再 OpenProcess。
- Job 在 spawn 前完成配置，`AssignProcessToJobObject` 在 spawn 后立即挂入。
- `IsProcessInJob` 作为挂入成功证据/测试辅助。
- `TerminateJobObject` 用于 kill 与 shutdown，终止已成功加入 Job 的成员。
- 所有 Win32 句柄走 RAII，重复 kill/shutdown 对已关闭句柄保持幂等。

### 生命周期集成

`ManagedProcess` 保存 Windows-only `JobHandle`。Job 在 spawn 前完成配置，spawn 后立即尝试挂入/验证，然后接管管道；任一步失败都执行：关闭/终止 Job、对 direct child 执行有限等待回收、返回包含 Win32 错误码的错误且不发布成功句柄。正常 kill/shutdown 先调用 `TerminateJobObject`，保证范围是已成功加入 Job 的成员；再使用现有 reap 逻辑清理 `Child` 与 map。最终 owner drop 释放 Job，kill-on-close 清理 Job 内仍存成员；自然退出后 Job 保留到显式清理，不提前改变背景进程语义。spawn→assign 窗口内产生的后代不在严格保证范围内。

方案 A 的已接受局限：`Command::spawn()` 与 `AssignProcessToJobObject()` 之间存在受调度影响、无固定时间上界的 race，子进程可能在挂入前创建未受管孙进程。该能力是纵深防御而非绝对安全边界；严格无窗口方案留作后续独立卡。

## 任务拆解

1. [x] 创建 ADR 0018 与 OpenSpec proposal/spec/design/tasks。
2. [x] 由 architect、security reviewer、consultant/test reviewer 对设计进行三路独立对抗审核，并采纳/记录意见。
3. [x] 增加 `windows-sys` 直接 Windows 依赖及最小 feature 集，确认 Cargo.lock 无非目标平台行为变化（实现证据见下）。
4. [x] 实现 Job RAII、spawn 挂入失败清理、以及对已成功加入 Job 成员的 kill/shutdown 终止（实现证据见下）。
5. [x] 增加 Windows 定向测试：创建孙进程并验证 Job 终止树、挂入失败清理/错误、重复 kill/shutdown 幂等、非 Windows 编译门禁（历史实现证据见下；当前精确 diff 复跑仍受环境门禁影响）。
6. [ ] 完成当前精确工作树的 core 回归、shared cargo check、version/OpenSpec/ADR 与格式验证；当前结果必须按 Passed / Not run / Blocked 分列，不能把历史快照写成全绿。
7. [ ] 完成收口对账、三路收口审核与稳定引用同步后，再归档本卡。

## 风险与回滚

- **P1 race**：spawn 到 assign 之间存在未受管窗口。验收中明确为已知残余，不宣称绝对 containment；若风险不可接受，回滚实现并另立严格 suspended-start 卡。
- **P1 极短命令兼容性**：子进程在 assign 前退出可能导致 start 返回错误；只有 assign/verify 成功才发布 handle，不以父退出推断安全。接受此 best-effort 局限；需要确定性失败与自然短命令回归。
- **P1 句柄泄漏**：Job 未纳入 ManagedProcess 或清理路径不完整。必须有 RAII 和重复清理测试。
- **P1 终止语义回归**：`kill` 结果、最终 poll、shutdown map 清理改变。保留现有顶层 Child 回收作为兜底并做生命周期回归。
- **回滚**：删除 Windows-only 模块/依赖并恢复 ProcessManager 原始 `Child::kill` 路径；不触碰 sandbox approval gate。

## 验收标准

1. Windows 编译通过，Job Object API 仅在 `cfg(windows)` 下编译；Linux/macOS core 测试与 check 不回归。
2. 每个 Windows managed child 创建独立 Job，设置 kill-on-close，且未设置 breakaway flags；挂入失败不会留下运行中的 child。
3. Job 内派生子进程后调用 `kill` 或 `shutdown`，父子进程均在有界时间内退出；重复调用幂等。
4. 进程自然退出后仍能 poll 最终状态，Job 句柄最终释放；错误路径不污染 process map。
5. `NoSandboxBackend`、`enforce_sandbox`、无人值守 Run fail-closed 语义不变。
6. 当前精确工作树的格式、core tests、shared cargo check、version/OpenSpec/ADR 验证必须逐项记录为 Passed / Not run / Blocked；不得引用不存在的 `npm run openspec:check`。

## 当前状态

- 2026-09-30：实现已落地于当前工作树，设计三路审核有条件通过；实现审核在批准的 best-effort 范围内 conditional pass。
- Gate0 收口状态：`Review`，closeout gate `Blocked`。阻塞不是功能实现未完成，而是当前精确 diff 归属、可复现验证与跨平台证据尚未闭合。
- 当前证据分层：version check 通过；canonical OpenSpec direct strict validation 54 passed；active PA-103 change failed；不存在 `npm run openspec:check`；cargo fmt 有预存全仓差异；精确定向测试受 target-test ACL 阻断；Linux/macOS 仅 CI 配置、未本机执行。
- 三路 Gate0 对抗审核均 PASS WITH FINDINGS / 有条件通过；已采纳既有状态枚举、证据分层和安全免责声明要求。

## Next Action

1. 冻结 PA-077 新功能开发，不启动 PA-044 或完整 SandboxBackend。
2. 先核对并隔离 PA-077 与工作树中 PA-102/治理/CI/脚本等其他变更的归属，不执行提交。
3. 修复或批准 `target-test` ACL 后，重跑原精确定向测试；按 Passed / Not run / Blocked 记录 raw evidence。
4. 等待 PA-103 active change 独立修复/关闭后再重跑全库 strict OpenSpec；本卡不修改或归档 PA-103。
5. 取得跨平台 CI 结果或明确其为未执行，再由三路实现/收口 reviewer 复核，完成 archive tasks 3.4 后才可转 Done。

## Resume Hint

下次先打开：

- `management/task-system/03_TASKS/PA-077-windows-job-object-containment.md`
- `management/task-system/02_REVIEWS/2026-09-30-pa077-gate0-closeout-review.md`
- `openspec/changes/archive/2026-09-30-windows-job-object-containment/tasks.md`
- `crates/pony-agent-core/src/agent/process.rs`

- Windows-only `windows-sys` Job RAII：private non-inheritable Job、`KILL_ON_JOB_CLOSE`、breakaway flags omitted、Child raw handle assign + `IsProcessInJob` verification、explicit tree termination and final-owner close fallback。
- Transactional start：setup/assignment/verification/pipe failure all return errors without publication; Windows paths use bounded kill/reap diagnostics. Per-manager DuplicateHandle + native `WaitForSingleObject` proves assign/verify/pipe failed-start direct child exit; deterministic wait-before-assign proves an already-exited child still fails closed. Exited-parent explicit `kill` and shutdown are both covered.
- Sandbox boundary unchanged：Job is lifecycle containment only, not `SandboxBackend`; autonomous Run remains fail-closed. Accepted residual is scheduler-dependent spawn→assign window with no strict containment claim. Nested host Job/breakaway empirical coverage and timer/outstanding-Arc stress coverage remain follow-up evidence, not advertised guarantees.
- Validation evidence (historical implementation snapshot, not a current full-green claim): Windows/core/process/tools/version/ADR evidence is recorded in the implementation review. The current tree must be revalidated before closeout.
- Current Gate0 evidence: `npm run version:check` passed; `npm run openspec:check` is not a repository script; direct strict validation reports 54 canonical specs passed but active PA-103 failed; `cargo fmt --check` has pre-existing repository-wide differences; the exact PA-077 targeted test rerun is blocked by `target-test\debug\.cargo-lock` access denied; Linux/macOS CI jobs are configured but not locally executed.
- Security boundary: Job is best-effort lifecycle/process-tree containment only. The scheduler-dependent spawn→assign race has no fixed bound and is not a security boundary; Job is not a filesystem/network/token/AppContainer/approval sandbox, does not register a SandboxBackend, and does not authorize autonomous Run. `NoSandboxBackend` and `enforce_sandbox` remain fail-closed.
- Independent fixed-snapshot correctness review: conditional pass within the approved best-effort scope; no remaining P0/P1 lifecycle blocker was identified, but closeout evidence is not yet closed.
