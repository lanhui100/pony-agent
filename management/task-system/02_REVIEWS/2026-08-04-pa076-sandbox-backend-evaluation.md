# PA-076 剩余工作项 4 — 真实 SandboxBackend 评估与接线决策（2026-08-04）

## 评估背景

PA-076 阶段 5 已落地 `sandbox.rs`：`SandboxSupportMatrix`（WindowsJobObject /
UnixProcessGroup / NoSandbox，`platform_default()` Windows→JobObject）、
`enforce_sandbox`（fail-closed 门禁）、`NoSandboxBackend`（恒 Unavailable）、
`TestSandboxBackend`（可配置）。governed dispatcher 在 `decision==Allow &&
requires_sandbox(descriptor)` 时若无 backend → `sandbox_unavailable`
（`crates/pony-agent-core/src/agent/dispatcher.rs:938-983`，`register_sandbox_backend`
在 line 532）。`process.rs` 的 `ProcessManager` 是纯生命周期后端
（start/poll/write_stdin/kill/shutdown），头注（line 22-24）明确 Job Object /
进程组 containment 归 sandbox 层，本模块不承诺。

本评估回答三个问题：
1. 一个真实的 Windows Job Object containment backend（禁 breakaway + kill-on-close
   + 句柄继承约束）需要哪些 Windows API / crate，哪些已在 Cargo 依赖树里。
2. 「完整 SandboxBackend」（文件/网络/环境隔离 + Job Object）相对「仅 Job Object
   containment」的增量工作量与风险。
3. 当前 fail-closed（`sandbox_unavailable`）为何是 design 合规终态；以及后续拆卡建议。

## 可行性与风险分析

### 2.1 Windows Job Object containment backend

**需要的 Win32 API**（全部在 `windows-sys` 0.61.2 的 `Win32_System_JobObjects`
feature 下确认存在，见「依赖证据」）：

| API | 用途 | 约束点 |
| --- | --- | --- |
| `CreateJobObjectW` | 创建 Job Object（无名） | 无对象名 → 不与其他 Job 命名冲突 |
| `SetInformationJobObject` + `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` | 设置限制 | 置位 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`；**不置位** `JOB_OBJECT_LIMIT_BREAKAWAY_OK`（2048）与 `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`（4096）→ 禁 breakaway |
| `AssignProcessToJobObject` | 把子进程挂进 Job | 在 `kill`/`shutdown` 前挂入；存在挂入前 race 窗口（见风险 R1） |
| `TerminateJobObject` | 整树终止 | `ProcessManager::kill`/`shutdown` 的强化：终止完整进程树而非仅顶层 PID |
| `IsProcessInJob` | 验证/证据 | 记录「该进程确已在受管 Job 中」的证明结果 |
| `CreateJobObjectW` 属性 + 句柄继承 | 句柄继承约束 | std `Command` 默认 `bInheritHandles=false`，仅三根 std 管道句柄可继承；如需显式 handle list 需 `STARTUPINFOEX`/`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` |

**std 集成的关键限制（风险点）**：Rust std `std::process::Command` 不暴露子进程的
原始 `HANDLE`（只有 `child.id()` PID），也不暴露主线程句柄。因此：

- **方案 A（简单，推荐用于 containment 卡）**：`Command::spawn()` 后立即
  `OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, pid)` →
  `AssignProcessToJobObject`。存在毫秒级 race 窗口：子进程可能在挂入前自产一个
  未受管孙进程。对 **containment（纵深防御）** 卡可接受——design.md Non-Goals 明确
  Job Object 只是 containment，从不是 sandbox/approval 的替代；真正的安全边界是
  dispatcher 的审批门禁（`sandbox_unavailable`）。须在卡文档中明确记录该 race 为
  已接受的局限。
- **方案 B（严格，工作量明显更高）**：`CommandExt::creation_flags` 置
  `CREATE_SUSPENDED` 挂起启动 → 挂入 Job（进程挂起故 PID 不会复用）→ 需
  `OpenThread(THREAD_SUSPEND_RESUME)` + `ResumeThread` 恢复（获取线程 ID 还需
  Toolhelp 快照 `CreateToolhelp32Snapshot` 或 `NtQueryInformationProcess`），或改走
  裸 `CreateProcessW` + `STARTUPINFOEX`。不推荐作为首个卡的默认路径。

**结论**：Job Object containment 后端**可行**，最小实现 = windows-sys 直接依赖 +
`Win32_System_JobObjects`（+ `Win32_System_Threading`/`Win32_Security`/
`Win32_Foundation`）feature + 方案 A 集成进 `ProcessManager` 的 Windows spawn 路径
（挂入时机、kill/shutdown 走 `TerminateJobObject`、`IsProcessInJob` 证据）。

### 2.2 「完整 SandboxBackend」的相对增量

| 维度 | 仅 Job Object containment | 完整 SandboxBackend 增量 | 风险 |
| --- | --- | --- | --- |
| 进程树 containment | 是（Job Object） | 复用 | 低 |
| 环境最小化 | 已在 `process.rs` 落地（`env_clear` + `ESSENTIAL_ENV_VARS` + allowlist） | 无增量 | 低 |
| 句柄继承约束 | 依赖 std 默认（够用） | 需 `STARTUPINFOEX`/handle list 显式化 | 中 |
| 文件/workspace 隔离 | 不做 | 规范化真实路径 + 拒绝越界（含符号链接/junction/设备路径 `\\.\`、`\\?\`）；OS 级强隔离需 AppContainer/受限令牌 | **高** |
| 网络隔离 | 不做 | 对任意子进程做网络约束需 WFP（Windows Filtering Platform）或 AppContainer；应用自身 fetch 已有 `web_access.rs` pinned-connector，但那是应用内路径，不是对任意子进程的约束 | **高** |
| 后端注册与 Run 启用 | 仍 `sandbox_unavailable`（approval 门禁未替换） | 注册为 dispatcher `SandboxBackend` 后无人值守 Run 才可用 | 门禁语义变更 |

**工作量判断**：仅 Job Object containment 是**一张聚焦卡**（一个 backend + spawn
接线 + 整树 kill + 测试）。完整 SandboxBackend（文件+网络+环境+Job Object）是
**多张卡的大工程**：文件隔离需面对符号链接/junction/设备路径逃逸，网络隔离需
WFP/AppContainer，且两者都需要真正的 OS 级强制而非路径字符串检查。

### 2.3 风险清单

- **R1**：post-spawn 挂入 race（方案 A）—— 挂入前孙进程逃逸窗口。缓解：记录为已接受
  局限；containment 仅纵深防御；approval 门禁是安全边界；可选方案 B 消除。
- **R2**：嵌套 Job（Win8+ 支持进程在多个 Job 中）——外层 Job 未置 breakaway 时子进程
  不能自行脱离，但需在测试中验证「子进程自建 Job」场景不会逃逸。
- **R3**：`TerminateJobObject` 是强终止，与 `ProcessManager` 现有「kill 后保留 entry 供
  poll 上报 Exited」语义需协调（整树退出码以顶层进程为准）。
- **R4**：完整沙箱的文件/网络 OS 级强制复杂度高，若误判会破坏既有 workspace 工具
  （search/glob/view_image 等读路径），需要独立卡与充分回归。
- **R5**：windows-sys 直接依赖需锁定 feature 集，避免与 tauri 等传递依赖的 feature
  合并产生意外膨胀（版本已存在，无新 crate）。

## fail-closed 终态的合规依据

当前无人值守 `Run` 在无真实 backend 时返回 `sandbox_unavailable`，正是 design 与
spec 要求的终态，而非未完成缺口：

1. **design.md Decision 7**：「无人值守 `Run` 只在真实 sandbox 可用时启用；无
   sandbox backend 的平台 fail closed。明确的 unsandboxed mode 只能逐次由 host 审批，
   并在结果和 trace 标为高风险，不能静默降级。」
2. **design.md Non-Goals**：「不以 command denylist、process group 或 Job Object
   代替 sandbox/approval；它们只可作为纵深防御或 containment。」→ 没有真实
   SandboxBackend 时，用 Job Object/进程组兜底**不是**合规替代，fail-closed 才是。
3. **design.md Risks**：「抽象 `SandboxBackend` 与 `ProcessBackend`；未实现真实
   sandbox 的平台禁用无人值守 Run，process group 只作为 best-effort containment。」
4. **spec `openspec/specs/process-tool-lifecycle/spec.md`**（及 change 内同名
   spec）「Autonomous Run SHALL require a real sandbox」：平台缺真实 sandbox 时
   「runtime SHALL fail closed 并返回 `sandbox_unavailable`」「SHALL NOT 退化为普通
   shell 或只使用 denylist/process group」。
5. **实现位置**：`dispatcher.rs:938-983` 无 backend → `sandbox_unavailable`；
   `sandbox.rs` spike 注记（line 9-12、22-27）已声明 Job Object 未实现为完整
   containment 且不替代审批门禁。→ **保持 fail-closed 即合规**，注册真实 backend
   是「新增能力」而非「修缺口」。

## 后续卡建议

### 卡 A（建议编号 PA-077）：Windows Job Object containment backend

- **范围**：仅进程树 containment（**不**替换 dispatcher 审批门禁、**不**启用无人值守
  Run）。实现：windows-sys 直接依赖 + `Win32_System_JobObjects` feature；spawn 后
  挂入非 breakaway + `KILL_ON_JOB_CLOSE` 的 Job；`ProcessManager::kill`/`shutdown`
  在 Windows 走 `TerminateJobObject`；`IsProcessInJob` 记录证明证据；决策记录方案 A
  的 race 为已接受局限（或实现方案 B 严格路径）。
- **前置依赖**：本评估裁决；`windows-sys = { version = "0.61", features =
  ["Win32_System_JobObjects", "Win32_System_Threading", "Win32_Security"] }` 加入
  `crates/pony-agent-core/Cargo.toml` 的 `[target.'cfg(windows)'.dependencies]`；
  明确 kill/shutdown 语义与 `ProcessManager` entry 保留协调。
- **验收要点**：Windows 测试证明 (a) 子进程启动孙进程后 kill 整树退出；(b)
  breakaway 被禁（子进程尝试脱离失败/无法脱离）；(c) Job close 时整树被杀；(d)
  证据字段记录 Job/containment 类型与证明结果；(e) 既有 `process.rs`/
  `sandbox.rs`/dispatcher fail-closed 测试全部保持绿。非 Windows 行为不变。
- **风险**：R1（post-spawn race，记录接受）、R2（嵌套 Job 逃逸验证）、R3（整树
  终止与 poll Exited 语义协调）。

### 卡 B（建议编号 PA-078 或并入沙箱里程碑）：完整 SandboxBackend

- **范围**：真实 `SandboxBackend`（workspace 文件隔离 + 网络隔离 + 环境 + 句柄继承
  + 复用卡 A 的 Job Object containment），注册进 dispatcher 后启用无人值守 Run。
- **前置依赖**：卡 A（进程树 containment）、文件/网络 OS 级强制方案选型（AppContainer
  受限令牌 vs WFP）、与既有 workspace 读路径（search/glob/view_image）回归策略。
- **验收要点**：`SandboxBackend::validate` 对越界 workspace/网络/环境请求拒绝；
  无人值守 Run 在受支持平台从 `sandbox_unavailable` 转为执行；unsandboxed host
  审批路径在结果与 trace 标高风险；失败保持 fail-closed。
- **风险**：R4（OS 级文件/网络隔离复杂度、对既有读路径的回归冲击）、R5（feature
  膨胀）、完整沙箱的跨平台不对称（Unix 侧仅 best-effort 进程组）。

## 依赖证据（已核实）

- `crates/pony-agent-core/Cargo.toml`：Windows target 直接依赖仅 `keyring`
  （windows-native）与 `winreg 0.56.0`；**无直接 windows-sys**。
- `src-tauri/Cargo.toml`：tauri 2 / window-vibrancy / raw-window-handle；**无直接
  windows-sys**。
- 根 `Cargo.lock`（workspace：crates/pony-agent-core + src-tauri）：存在
  `windows-sys 0.45.0 / 0.59.0 / 0.60.2 / 0.61.2`（传递依赖，经 tauri、native-tls、
  schannel、softbuffer、window-vibrancy 等），以及 `windows 0.61.3`。`src-tauri/
  Cargo.lock` 同样含 `windows-sys 0.61.2`。
- `windows-sys` 0.61.2 的 `Cargo.toml` 定义 `Win32_System_JobObjects` feature（→
  `Win32_System`）；`src/Windows/Win32/System/JobObjects/mod.rs` 确认导出
  `CreateJobObjectW`、`SetInformationJobObject`、`AssignProcessToJobObject`、
  `TerminateJobObject`、`IsProcessInJob`、`JOBOBJECT_EXTENDED_LIMIT_INFORMATION`、
  `JOB_OBJECT_LIMIT_BREAKAWAY_OK`(2048)、`JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`(4096)、
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`(8192)。
- 结论：**Job Object containment 所需 Win32 API 已在依赖树内**（传递存在），实现只需
  在 core crate 加一条直接依赖 + 开启相应 feature，**零新增 crate**。

## 结论

- 真实 Windows Job Object containment 后端**可行**，依赖/API 证据充分，建议拆为
  PA-077 单独落地；完整 SandboxBackend 是更大工程，建议拆为独立卡（PA-078 或沙箱
  里程碑）。
- 本卡**保持 fail-closed（`sandbox_unavailable`）为设计合规终态**：design.md
  Decision 7 / Non-Goals / Risks 与 process-tool-lifecycle spec「Autonomous Run
  SHALL require a real sandbox」均明确「无真实 sandbox 的平台禁用无人值守 Run，
  不得静默降级」，Job Object/进程组只作 containment，不替代审批门禁。
- 决策已落到：`docs/architecture/tool-runtime-descriptor-registry.md`
  「Two Remaining Integrator Notes」第 2 条（已裁决）与
  `crates/pony-agent-core/src/agent/sandbox.rs` 模块头注记（2026-08-04）。
