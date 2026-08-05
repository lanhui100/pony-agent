# PA-076 Phase 5 Review — Process 生命周期与沙箱门禁（task 5.7）

## 审核对象

- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/design.md)（Decision 7、Verification Strategy line 140 验收项、Non-Goals）
- 阶段 5 产物：
  - `crates/pony-agent-core/src/agent/process.rs`（ProcessManager：start/poll/write_stdin/kill/kill_after/shutdown、并发排空、bounded buffer、truncation/dropped-byte 证据、最小环境 env_clear+allowlist）
  - `crates/pony-agent-core/src/agent/sandbox.rs`（SandboxSupportMatrix/enforce_sandbox/NoSandboxBackend/TestSandboxBackend，模块头含 2026-08-04 裁决记录）
  - `crates/pony-agent-core/src/agent/dispatcher.rs:939-984`（sandbox 门禁）、`:531-539`（register_sandbox_backend）
  - `crates/pony-agent-core/src/agent/tools.rs`（workspace_run_command 走 ProcessManager；legacy run 门禁 :1383-1401）
  - `crates/pony-agent-core/src/agent/tool_runtime.rs`（SandboxBackend/ProcessBackend ports、ProcessPollResult、SandboxRequest）
- 相关：`crates/pony-agent-core/src/agent/governed_executor.rs`、`crates/pony-agent-core/src/agent/runtime/mod.rs`（默认 executor）、`crates/pony-agent-core/src/agent/control_plane/mod.rs:924`
- 上轮评估记录：`management/task-system/02_REVIEWS/2026-08-04-pa076-sandbox-backend-evaluation.md`

审核为只读；未运行完整测试套件，测试证据以 8.3 门禁全绿为准。

## 审核阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@security-reviewer` | **CONDITIONAL PASS** | 最小环境接线反转（env_clear 仅当 allowlist 非空时触发，而生产两处构造点恒传空 allowlist）；legacy ToolRouter Run 在无 backend 时静默降级为普通 shell；host-approved-unsandboxed 高风险标记缺失；丢弃字节数未透出 | **P1-1 本轮修复**；P1-2 落地显式 opt-in 或记录承接；P2-1/P2-5 记录承接 |
| `@performance-reviewer` | **CONDITIONAL PASS** | run_command 跨 run 累计输出无界；无死锁/无界缓冲/阻塞 kill 边界均已核实为健壮 | P2-2 记录承接；P3 接受 |

无 P0。两份视角均确认：生产默认 executor 为 governed（fail-closed），runtime 默认 `Run` 无 sandbox backend 时返回 `sandbox_unavailable`（`runtime/mod.rs:446-447`、`dispatcher.rs:939-983`、测试 `runtime/mod.rs:9984`）；ProcessManager 锁顺序一致无死锁；每流内存有界；cross-session handle 拒绝生效；containment 被正确声明为纵深防御而非安全边界。

## 已核实的健壮点

1. **fail-closed 默认路径成立**：`AgentRuntimeBuilder::build` 默认 executor 为 `build_governed_executor`（`runtime/mod.rs:446-447`）；`HostControlPlane::build` 默认 `AgentRuntime::new()`（`control_plane/mod.rs:924`）。governed 路径对 `Run`（`ToolKind::Execute`，`tools.rs:4445`）命中 `requires_sandbox`（`dispatcher.rs:997-1003`），无 backend → `sandbox_unavailable`（`dispatcher.rs:945-953`）；`GovernedToolExecutor::execute` → `dispatch_governed`（`dispatcher_composites.rs:102`），门禁先于 handler。
2. **无死锁**：`poll_inner`（process.rs:342-392）与 `terminate`（:418-442）锁序一致（state → child → stdin，stdin 均临时持有）；`write_stdin`（:394-413）state 释放后再锁 stdin；shutdown 先收集中止目标再逐个 terminate/remove。未发现锁序反转。
3. **每流内存有界**：`BoundedBuffer::push` 单次 push 后 `drain(..excess)`（process.rs:84-95），data 长度恒 ≤ cap + 8192（单 chunk）；不会因子进程洪泛而无界增长，drain 线程不阻塞子进程。
4. **句柄 session-bound**：`lookup`（process.rs:460-476）校验 `entry.session_id == session_id`，跨 session 拒绝（poll/write_stdin/kill 均经 lookup；测试 process.rs:743-764）。
5. **kill 后 entry 保留供 poll 上报 Exited**：`terminate` 置 `ExitState::Exited` 后保留 entry（process.rs:415-442），幂等二次 kill 清理；测试 process.rs:707-731、827-842。
6. **截断证据**：`truncated` 标志 sticky、`dropped` 计数累计（process.rs:99-107）；`drain_stats` 提供计数证据；大输出测试 process.rs:766-795。
7. **containment 声明为纵深防御而非安全边界**：`sandbox.rs:9-12`、`:22-27` 与 `SandboxSupportMatrix::WindowsJobObject` 文档（:32-38）明确「Job Object 不替代 SandboxBackend 审批门禁」；裁决记录落于 `sandbox.rs` 头注与 `docs/architecture/tool-runtime-descriptor-registry.md:309-333`。
8. **测试覆盖良好**：sandbox denial（dispatcher.rs:2799、governed_executor.rs:378、tools.rs:6858）、环境隔离（process.rs:548）、跨 session 拒绝（:743）、大输出（:766）、stdin 及 exit 后拒绝（:798/:817）、非零退出（tools.rs:6799）、timeout（tools.rs:6826）、cancel（process.rs:827）、kill_after（:845）、shutdown session 隔离（:860）、stale handle fail-closed（:726）、unknown handle（:734）。

## 发现与分级

### P0

无。

### P1

#### P1-1 最小环境接线反转：sandboxed 子进程永不触发 env_clear，执行路径恒继承完整父环境

- **位置**：`process.rs:275`（`if !request.sandbox.environment_allowlist.is_empty()` 才执行 `env_clear`）；`dispatcher.rs:965-972` 与 `tools.rs:1387-1391`（两处 `SandboxRequest` 构造恒传 `environment_allowlist: Vec::new()`）。
- **失败场景**：design.md Decision 7 明确「子进程使用最小环境，不能继承 provider key、session secret 或 ambient proxy」。但 `env_clear` 被非空 allowlist 门控，而生产所有构造点都传空 allowlist → 门控恒不满足 → 任何实际执行的子进程都继承完整父环境。当前无人值守 Run 被 fail-closed 挡住、仅有 legacy 迁移路径在跑（本就继承完整父环境，已文档化）；一旦 PA-077/078 注册真实 backend，governed `Run` 会经 `dispatcher.rs:965` 构造空 allowlist 的请求放行执行，**所谓「sandboxed」子进程仍继承完整父环境**——此时该缺陷从「潜在」变为真实密钥泄露。语义上 `environment_allowlist` 的自然读法是「要保留的变量（追加式）」，空列表应表示「仅保留 essential」，实现却把空列表当作「全部继承」——**契约反转**。
- **证据缺口**：唯一环境隔离测试（`process.rs:548-577`）直接调用 ProcessManager API 并传非空 allowlist，未经过生产请求构造点（dispatcher/tools），因此发现不了接线断裂。
- **建议处置**：把「sandboxed」与「legacy 全继承」做成显式标志（例如 `SandboxRequest` 增加 `isolate_environment: bool`，或 process.rs 对空 allowlist 走 env_clear+`ESSENTIAL_ENV_VARS`、非空条目再追加），并让 dispatcher/run_command 两个构造点在「受管 Run」时置位；补一条集成测试：注册 available backend → governed Run 执行 `set`，断言父进程 sentinel 不泄漏（与 process.rs:552 同一手法）。

#### P1-2 legacy ToolRouter `Run` 路径在无 backend 时静默降级为普通 shell

- **位置**：`tools.rs:1383-1401`（`sandbox_backend: None` 时跳过 `enforce_sandbox`）、模块头 `tools.rs:9-13`（文档化迁移窗口）。
- **失败场景**：任何 runtime 显式以 legacy `ToolRouter` 作为 `tool_executor`（写法即 `control_plane/mod.rs:2961-2970` 测试辅助），无人值守 `Run` 会在无 sandbox backend 时以完整父环境执行，仅靠 `denied_run_command_reason` 黑名单（tools.rs:4636+）兜底——而 design.md Non-Goals 明确 denylist 不替代 sandbox/approval。这直接违反「无 sandbox backend 的平台 fail closed，不能静默降级」。当前生产默认 governed、无生产 legacy 用法，故非即时暴露，但该路径是活的公共 API。
- **建议处置**：把 `ToolRouter` 默认改为 fail-closed（默认注册 `NoSandboxBackend`，让 legacy `run_command` 无 backend 时也返回 `sandbox_denied`），把「迁移期无门禁」改为显式 `legacy_without_sandbox_gate()` opt-in；或至少在阶段 5 收口记录该路径的移除 deadline（runtime 切换后删除）。若本卡裁决为「文档化迁移窗口、接受」，需在结论中显式记录承接点。

### P2

#### P2-1 丢弃字节数未透出（design.md Decision 7 偏离）
- **位置**：`process.rs:370-379` 把 `_stdout_dropped`/`_stderr_dropped` 丢弃；`ProcessPollResult`（`tool_runtime.rs:259-267`）无 dropped 字段；`run_command` 输出（`tools.rs:1499-1527`）仅 `stdoutTruncated`/`stderrTruncated`；`drain_stats`（process.rs:244-257）存在但仅测试使用。
- **失败场景**：Decision 7 要求「输出返回截断标记**和丢弃字节数**」。消费者（模型/runtime）只能知道「有截断」，无法量化丢失量；`run_command` 结束后 entry 即被清理，`drain_stats` 事后不可用。
- **建议处置**：`ProcessPollResult` 增加 `stdout_dropped_bytes`/`stderr_dropped_bytes`，`run_command` 输出透出；或至少把计数并入 Run 结果。

#### P2-2 `run_command` 跨 run 累计输出无界（性能）
- **位置**：`tools.rs:1444-1445`（`stdout.push_str`/`stderr.push_str` 贯穿整个 run）；governed 路径设 `unbounded_output_and_deadline: true`（`governed_executor.rs:146-149`）。
- **失败场景**：`ProcessManager` 每流缓冲有界（cap=64KiB）只约束「单次 poll 返回量」，不约束 run 总累计。20ms poll 间隔下每 poll 至多 ~64KiB，120s timeout 的持续输出命令可累计约 300MB+ String（约 3.2MB/s），长时高输出命令存在内存膨胀。
- **建议处置**：在 Run 工具聚合层加总输出字节预算（截断并置 `stdoutTruncated`），或复用 process 缓冲上限的累计语义。

#### P2-3 Exited 边界输出证据可能丢失
- **位置**：`process.rs:366-368`（`wait_for_drain` 至多等 500ms）；`tools.rs:1478`（Exited 后立即 `kill` 清理 entry）。
- **失败场景**：子进程退出但 drain 线程 500ms 内未达 EOF（孙进程持有管道写端）时，`poll` 返回不完整输出且 `truncated` 可能仍为 false；run_command 随即 kill 删除 entry，尾部字节与截断证据一并丢失、无任何标记（`large_output_is_truncated_with_evidence` 测试注释 process.rs:773-775 亦承认该时序）。
- **建议处置**：Exited 时若 drain 未完成，保留 entry（或返回 `drain_incomplete` 标志）直到两个 `done` 置位或明确超时。

#### P2-4 dispatcher 层 sandbox 门禁 accept/deny/host-approved 分支无测试
- **位置**：门禁 `dispatcher.rs:939-984`；唯一测试 `execute_descriptor_without_sandbox_backend_fails_closed`（dispatcher.rs:2799-2817）。
- **失败场景**：available backend + `validate` Ok → 放行执行、available + `validate` Err → `sandbox_denied`、`HostApprovedUnsandboxed` → 放行——三个分支均无 dispatcher 级测试（sandbox.rs 单测只覆盖 `enforce_sandbox`，tools.rs 只覆盖 legacy run_command 路径）。
- **建议处置**：补 dispatcher 级测试（用 `TestSandboxBackend` 注册到 `register_sandbox_backend`），断言各分支错误码与放行语义。

#### P2-5 host-approved-unsandboxed 高风险标记未实现
- **位置**：`tools.rs:1519-1526` 输出硬编码 `"hostMediated": false, "approvalMode": "none", "decisionSource": "runtime"`；无 trace 高风险标记。
- **失败场景**：design.md Decision 7「明确的 unsandboxed mode 只能逐次由 host 审批，并在结果和 trace 标为高风险」。`SandboxAvailability::HostApprovedUnsandboxed`（tool_runtime.rs:224）放行后，结果与 trace 均无高风险标注（当前无真实 backend、路径不可达，故为契约缺口而非即时漏洞）。
- **建议处置**：PA-077/078 接线时实现：放行路径输出 `hostApprovedUnsandboxed: true` + 高风险 trace 事件。

### P3

- **P3-1** `kill_after` 每 run 一线程（`process.rs:209-217`）：timer 线程存活至 `duration` 到期；同步 run_command 下并发数量受「timeout 窗口内 run 数」约束，非真实泄漏，但可改为共享 timer/每 session 扫描。
- **P3-2** 无「平台 containment canary」测试（design.md 验收项 line 140）：Job Object/进程组 containment 未实现，canary 属 PA-077 验收要点；需在承接卡中显式列出。
- **P3-3** `ProcessManager::shutdown`（`process.rs:221-239`）无任何生产调用方：run_command 逐 run 用 `kill` 清理，但会话关闭从不触发进程级清理；宿主进程崩溃时在途 Run 子进程孤儿化（无 Job Object kill-on-close，直到 PA-077）。
- **P3-4** legacy `run_command` 使用合成 session id `legacy-run-{nanos}`（`tools.rs:1405`），未绑定 runtime 真实 session；Decision 7 的 `process_start/process_poll/process_write_stdin/process_kill` 原语未暴露为工具，handle 从不向模型可见——契约的 session/run/owner 绑定语义目前只在 `ProcessBackend` port 层体现。
- **P3-5** timeout 竞态：`kill_after` 计时器与 `run_command` deadline 轮询（`tools.rs:1436-1444`）同时触发时，若 poll 先观察到 timer 造成的 Exited，会报 `non_zero_exit` 而非 `timeout`（窄窗口、确定性受损）。
- **P3-6** 测试环境变量全局性：`process.rs:552/568` `set_var`/`remove_var` 操作进程全局 env，并行测试下可能互相干扰（run_command 侧无断言 sentinel 缺失，故无实际冲突，属卫生问题）。

## 测试覆盖矩阵

| 验收维度（design.md line 140） | 覆盖 | 缺口 |
| --- | --- | --- |
| sandbox support matrix | `sandbox.rs` tests（platform_default / NoSandboxBackend / TestSandboxBackend / enforce_sandbox） | — |
| 跨 workspace/网络/环境隔离 | env 隔离单测（process.rs:548，直连 API） | workspace/网络隔离属 PA-078 完整沙箱；**生产请求构造点的 env 接线无测试（P1-1）** |
| 跨 session handle 拒绝 | process.rs:743-764 | — |
| 大输出无死锁 | process.rs:766-795（截断+dropped 证据） | run_command 总累计无界（P2-2）；Exited 边界证据（P2-3） |
| stdin | process.rs:798/:817 | legacy run_command 无 stdin 参数（决策可接受） |
| 多轮 poll | `poll_until_exited` + `long_lived_reports_running`（process.rs:707） | — |
| timeout | tools.rs:6826（run_command）、process.rs:845（kill_after）、:827（cancel） | timeout 竞态确定性（P3-5） |
| session close | process.rs:860（shutdown 隔离性） | `shutdown` 未接线到真实 session 关闭（P3-3） |
| 平台 containment canary | — | 无（P3-2，承接 PA-077） |
| 输出预算（截断证据） | process 缓冲 cap | Run 结果不包含丢弃字节数（P2-1）；aggregation 无预算（P2-2） |

## 结论

**CONDITIONAL PASS**。

- **P1-1（最小环境接线反转）为硬条件**：在注册真实 backend 前必须修复（或至少将 dispatcher/tools 两处 `SandboxRequest` 构造与 `process.rs` env_clear 门控对齐到 Decision 7），否则 PA-077/078 落地即产生 sandboxed 子进程泄漏 provider key/session secret 的安全漏洞。
- **P1-2（legacy 静默降级）**：当前生产默认 governed、无暴露面，接受为文档化迁移窗口的前提是记录移除/opt-in 承接点；若裁决为「保持现状」需在本卡结论显式背书。
- 其余 P2/P3 记录并绑定承接：P2-1/P2-4/P2-5 → PA-077/078 沙箱接线；P2-2/P2-3/P3-1/P3-5 → Run 迁移收口；P3-3 → 会话持久化收口；P3-2 → PA-077 验收要点。
