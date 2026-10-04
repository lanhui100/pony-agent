# Pony Agent 核心工具拓展包 (PA-104 至 PA-111) 系统架构蓝图 (System Blueprint)

**状态**: 已冻结  
**日期**: 2026-10-02  
**架构负责人**: Team Lead (协同参谋团: advisor-product, advisor-arch, advisor-eng-risk)  
**关联版本/里程碑**: Phase 8 / v0.2.0  

---

## 1. 目标与产品边界 (Scope & Non-Goals)

### 核心交付目标 (Goals)
1. **执行吞吐与无阻塞体验（PA-104 / PA-105）**：赋能 Agent 无缝处理交互式命令行与后台长驻作业（编译/测试/服务启动），主 Turn 脱离 120s 阻塞限制，进程绑定系统级沙箱树（Windows Job Object / POSIX pgid），杜绝孤儿与僵尸进程。
2. **精细语义理解与上下文降本（PA-106 / PA-107 / PA-108）**：通过标准 Unified Diff（`apply_patch`）实现低 Token 消耗与零行号漂移的文件原子写入；通过只读 LSP 智能客户端提供定义跳转、引用查询与编译诊断；通过模糊匹配与本地 remote diff 极速定位变更。
3. **长程自驱与团队协同闭环（PA-109 / PA-110 / PA-111）**：提供跨多轮持久化的 Goal 容器与有向无环依赖阻断校验；支持按需派生上下文隔离的 Subagent/Teams 与写域（write_scopes）锁控制；显式声明终态 Deliverables（`present`）并在桌面端提供原生交互卡片。

### 明确非目标 (Non-Goals)
1. **不做云端/跨宿主远程 PTY 集群（PA-104 / PA-105）**：终端与 Job 严格限定在本机工作区与当前机器进程树，不提供远程 SSH 协议簇、外部多节点容器调度或分布式后台作业队列。
2. **不做通用全功能 IDE 与自研语言分析器（PA-106 / PA-107）**：LSP 仅实现 JSON-RPC Client 转发并只读查询，不内置 Parser/Linter，不尝试自动静默安装系统语言环境；`apply_patch` 仅支持标准 Unified Diff 格式的原子落盘，不承担 AST 级冲突合并。
3. **不做无限自治黑盒与深度派生（PA-109 / PA-110）**：Goal 强制设定最大轮数上限（Max Rounds ≤ 100）与人工干预断点；Subagent 派生深度严格限制（`depth <= 2`），单会话并发智能体上限 ≤ 4，不支持去中心化的无限自我繁衍。
4. **不做自包含富文本/Office 编辑器（PA-111）**：`present` 仅负责工作区正规物理文件的元数据注册、类型白名单校验与前端卡片呈现，文件直接由宿主系统原生应用打开。

---

## 2. 系统拓扑与模块契约 (Topology & Contracts)

### 系统拓扑
```
┌────────────────────────────────────────────────────────────────────────┐
│ 表现层 (Presentation Layer / Vue 3 + Pinia)                           │
│  - 纯状态呈现与用户指令下发，严禁直连模型与旁路外部网络                │
│  - Xterm.js 虚拟终端 / Diff 视图 / 成果物卡片 / 目标进度树 / 协作拓扑  │
└───────────────────────────────────▲────────────────────────────────────┘
                                    │ IPC (Tauri Event: push / Command: req-res)
┌───────────────────────────────────▼────────────────────────────────────┐
│ IPC 适配与流控层 (tauri_adapter / Tauri Commands)                      │
│  - 双向通道生命周期管理（Terminal PTY 流 / Job 流式回写 / Event 推送） │
│  - 输入校验、Token 级限速、会话鉴权、前端销毁时连带释放清理            │
└───────────────────────────────────▲────────────────────────────────────┘
                                    │ 异步调用 (Tokio channels / mpsc / broadcast)
┌───────────────────────────────────▼────────────────────────────────────┐
│ 核心调度与编排层 (crates/pony-agent-core)                              │
│  - Tool Registry & Dispatcher (工具注册、参数反序列化、动态路由)      │
│  - Runtime State Machine (TurnRunner / GoalEngine / TeamOrchestrator)  │
│  - Local Intelligence Subsystems (LSP Client Manager / Patch Engine)   │
│  - Trace & SQLite Persistence (Event Sourcing ADR-0008, 审计轨迹)      │
└───────────────────────────────────▲────────────────────────────────────┘
                                    │ 安全上下文穿梭 (Sandbox Guard / Path Validator)
┌───────────────────────────────────▼────────────────────────────────────┐
│ 基础设施与系统运行时隔离层 (OS Runtime & Sandboxing)                    │
│  - Process/Job Containment (Windows Job Object ADR-0018 / POSIX pgid)  │
│  - Cross-platform PTY (portable-pty / ConPTY / openpty)                │
│  - Fail-Closed 文件安全层 (Path Canonicalize / Workspace Jails)        │
└────────────────────────────────────────────────────────────────────────┘
```

### 核心模块职责与边界
- **PTY 终端管理器 (`terminal_manager`)**：跨平台终端抽象，基于 `portable-pty`，Windows 下挂载 private Job Object，POSIX 下绑定进程组与 PDEATHSIG，负责 2MB 环形内存缓冲与双向流。
- **后台作业管理器 (`job_manager`)**：异步长任务管控，支持作业挂起、心跳、增量日志游标读取（`job_output`）及内核级强杀（`job_kill`）。
- **补丁引擎 (`patch_engine`)**：解析 Unified Diff，基于文件内容哈希进行基线校验，纯内存预检（Dry-run），验证通过后执行全有或全无（All-or-Nothing）原子替换。
- **LSP 客户端池 (`lsp_pool`)**：基于 JSON-RPC 2.0 维持与本地 language server 的 stdio 通信，单例懒加载、空闲 TTL 自动析构与 3 次崩溃熔断保护。
- **代码感知检索器 (`workspace_searcher`)**：纯 Rust 遍历（结合 ignore crate），路径模糊打分截断（上限 10 万节点），Git 外部命令严格参数白名单过滤。
- **长程目标引擎 (`goal_engine`)**：跨 Turn 目标容器，支持 DAG 无环依赖校验、状态单调迁移与 SQLite 事件溯源持久化，防连续停滞熔断。
- **多智能体编排器 (`team_orchestrator`)**：管理 Subagent 隔离执行、深度防炸弹（depth ≤ 2）与共享看板写域（`write_scopes`）CAS 乐观锁。
- **成果物注册表 (`deliverable_registry`)**：严格工作区物理路径校验、MIME 嗅探与前端卡片呈现。

---

## 3. 核心数据模型与状态流转 (Data & State)

### 核心实体/Schema

```rust
// 1. PTY 会话
pub struct TerminalSession {
    pub id: Uuid,
    pub pty_pair: Box<dyn MasterPty + Send>,
    pub child_process: Box<dyn Child + Send + Sync>,
    pub ring_buffer: Arc<RwLock<RingBuffer>>,
    pub created_at: Instant,
}

// 2. 后台作业
pub struct BackgroundJob {
    pub id: String,
    pub command: String,
    pub pid: u32,
    pub status: JobStatus, // Queued, Running, Terminating, Terminated(ExitStatus)
    pub log_spill_path: PathBuf,
    pub total_bytes_written: u64,
}

// 3. 长程目标
pub struct GoalEntity {
    pub id: String,
    pub revision: u64,
    pub objective: String,
    pub status: GoalStatus, // Draft, Active, Paused, Completed, Blocked
    pub blocked_reason: Option<String>,
    pub max_rounds: u32,
    pub current_round: u32,
}

// 4. 团队任务与写域
pub struct TeamTask {
    pub id: String,
    pub subject: String,
    pub owner: Option<String>,
    pub status: TaskStatus, // Pending, InProgress, Completed
    pub blocked_by: Vec<String>,
    pub write_scopes: Vec<PathBuf>,
    pub revision: u64,
}

// 5. 交付物声明
pub struct DeliverableArtifact {
    pub path: PathBuf,
    pub description: String,
    pub mime_type: String,
    pub size_bytes: u64,
}
```

### 关键状态机流转

```
[Goal 状态流转]:
Draft ──create_goal──> Active ──pause/breakpoint──> Paused ──resume──> Active
                          │
                          ├──update_goal(complete)──> Completed (终态)
                          └──eval(blocked/rounds)───> Blocked (需人工干预)

[Job 状态流转]:
Queued ──spawn──> Running ──exit(code)──> Terminated(Exited)
                     │
                     └──job_kill/timeout──> Terminating ──force_reap──> Terminated(Killed)

[Patch 状态流转]:
Diff Input ──parse──> ParseHunks ──dry_run──> In-Memory Validated ──atomic_swap──> Committed
                         │                         │
                         └──parse_error (Rollback) └──version_mismatch (FS_STALE_VERSION)
```

---

## 4. 关键技术选型与 ADR 索引 (Decisions)

| 领域 | 选型结果 | 核心考量 | 对应决策记录 (ADR) |
| :--- | :--- | :--- | :--- |
| **终端 PTY 抽象** | `portable-pty` + Win Job / POSIX pgid | 跨平台 ConPTY/openpty 一致抽象，防孤儿进程 | [ADR-0020](../decisions/0020-cross-platform-pty-and-containment.md) |
| **原子补丁引擎** | 纯 Rust Unified Diff 内存两阶段应用 | 杜绝大段文本正则漂移，省 Token，全有或全无原子性 | [ADR-0021](../decisions/0021-atomic-patch-application-contract.md) |
| **代码智能 (LSP)** | JSON-RPC 2.0 stdio 客户端池 + 3次熔断 | 只读查询防破坏，单例池 + 资源配额防雪崩 | [ADR-0022](../decisions/0022-lsp-client-lifecycle-and-resilience.md) |
| **长程目标持久化** | SQLite Event Sourcing (ADR-0008 演进) | 目标状态迁移纳入事件日志，断点续传可严格复原 | [ADR-0023](../decisions/0023-goal-engine-event-sourcing.md) |
| **多智能体写域隔离** | CAS 乐观锁 + `write_scopes` 依赖声明 | 规避并发文件写入撕裂与上下文过度下发 | [ADR-0024](../decisions/0024-multi-agent-write-scope-concurrency.md) |

---

## 5. 任务分解映射 (Backlog Decomposition Map)

将本蓝图切割为 8 个高内聚、独立可验证的事项（按依赖关系依次推进，WIP=1）：

| 阶段 / 事项 ID | 任务名称 | 范围契约与交付标准 | 依赖关系 |
| :--- | :--- | :--- | :--- |
| **Stage 1 (PA-104)** | 交互式持久终端 PTY 工具族 | 实现 `terminal_open/send/read/signal/close`，跨平台 PTY 隔离与 RingBuffer，进程零泄露 | 继承 PA-077 进程树基础设施 |
| **Stage 2 (PA-105)** | 异步后台作业生命周期工具族 | 实现 `job_start/output/kill/list`，支持流式日志文件转储与增量游标消费，强杀级联清理 | 依赖 PA-104 终端与进程底座 |
| **Stage 3 (PA-106)** | 原子代码补丁应用工具 | 实现 `apply_patch`，解析 Unified Diff，沙箱路径穿越与换行符校验，双阶段全量原子提交/回滚 | 无（独立纯 Rust 核心引擎） |
| **Stage 4 (PA-107)** | 语言服务器协议（LSP）智能工具 | 实现 `lsp_definition/references/hover/diagnostics` 只读查询，stdio JSON-RPC 通信与 3 次崩溃熔断 | 无（独立客户端引擎） |
| **Stage 5 (PA-108)** | 极速模糊文件检索与 Git 差异工具 | 实现 `fuzzy_file_search` 与 `git_diff_remote`，纯 Rust 树遍历，参数注入防护与结果截断 | 无（独立工具集） |
| **Stage 6 (PA-109)** | 长程自驱目标工具族 | 实现 `create_goal/get_goal/update_goal`，DAG 依赖校验，SQLite 事件溯源重放与防死锁停滞熔断 | 依赖 ADR-0008 事件底座 |
| **Stage 7 (PA-110)** | 智能体派生与协作编排工具族 | 实现 `subagent`（树深度≤2）、`workflow` DAG 管道与 `team_task_*` 协作看板，CAS 写域防冲突 | 依赖 PA-105/PA-109 任务底座 |
| **Stage 8 (PA-111)** | 结构化成果物声明工具 | 实现 `present` 工具与前端卡片呈现，严格工作区合法性与文件类型白名单校验，原生打开动作 | 依赖 Core 工具分发面 |
