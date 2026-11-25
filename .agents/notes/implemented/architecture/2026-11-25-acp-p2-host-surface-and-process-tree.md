# Agent Note: ACP P2 宿主面（Tauri 三命令）+ Windows 子进程树纳管 + cancel-race 归约

Status: implemented

## Problem

三件 ACP 遗留，收口前都是"契约不成立但被别的层兜住"：

- `backlog:B163`（P3）：ACP 只有 `ponycli acp` 一种宿主形态。桌面端既列不出可用外部 agent，
  也没法自己发起/取消一次委派——`acp_delegate` 是**模型侧**工具调用，界面没有入口。
  §4 P2 表把这三条 Tauri 命令标成"可选"，因此一直未落地。
- `backlog:B165`（P3）：`AcpTransport::spawn` 在 Unix 用 `process_group(0)` + 进程组 SIGKILL
  覆盖 agent 派生的孙进程；Windows 侧 `close()` 只 `child.kill()`，孙进程残留到自然结束。
  根因是 `agent/process/windows_job.rs` 的 `Job` 被收紧成 `pub(super)`，ACP 侧拿不到。
- `backlog:B169`（P3）：`AcpAgentSession::prompt` 只在 `Err` 分支看取消事实。agent 收到
  `session/cancel` 后若仍以 `end_turn` 收尾（cancel-race，实测 600ms 取消 + 6.5s agent ⇒
  `status=ok/6628ms`），本轮会被归约成**成功**。生产路径靠 dispatcher 兜住，但会话层契约
  不成立，界面可能把"已取消的一轮"显示成"成功的一轮"。

## Decision

**1) 新增 `acp::host`（`crates/pony-agent-core/src/acp/host.rs`）作为唯一宿主编排点**

- `list_agents()`：`config::load_agents()` → `AcpAgentSummary`。摘要**只回 env 键名**，
  不返回值（清单里可能有代理凭据），`builtin` 走 `config::is_builtin_agent`（与
  `ponycli acp --list` 同一真源，CLI 侧原有本地判定已删除）。
- `run_delegation(request, sink)`：解析清单 → 校验 cwd（绝对 + 存在 + 是目录 + canonicalize）
  → 登记取消令牌 → `AcpAgentSession::spawn` → `set_turn_budget` / `set_cancellation_token`
  → `prompt` → `close`。返回 [`ToolResult`]，错误 JSON 复用工具面的 `error_result`
  （新增 `pub(crate)` 再导出），前端不需要第二套错误码。
- `cancel_delegation(id)`：只翻登记表里的 `CancellationToken`，终局由归约结果表达。
- `delegation_id` 由**调用方**分配（前端 `crypto.randomUUID()`）；重复登记按
  `acp_delegate_duplicate_id` fail-closed，而不是静默覆盖取消目标。

**2) Tauri 三命令只是透传 + 事件出口**

`src-tauri/src/lib.rs` 增 `acp_list_agents` / `acp_delegate_start` / `acp_delegate_cancel`
并注册进 `invoke_handler`；`start` 走 `BlockingHelper::spawn`（不占 IPC 线程）并装配
`tauri_adapter::TauriAcpDelegateSink`，事件名 `acp://delegate`，负载带 `delegationId`
归属。这里**不出现** JSON-RPC、进程管理或状态机。

**3) `acp::process_tree::ProcessTree` 统一回收口径**

Windows 上复用 `agent::process::windows_job::Job`（模块与 `Job` 可见性提升到 crate 级，
新增 `create_default()` 收口 `#[cfg(test)]` 注入参数），spawn 后立即纳管并校验成员关系；
失败即回滚子进程并报 `acp_spawn_failed`（fail-closed，不降级成"只杀直接子进程"）。
Unix 是本类型的零字节占位——进程组语义已在 spawn 时建立，调用点不需要 cfg 分支。

**4) `prompt` 的 Ok 分支：取消事实优先于 stopReason**

命中 `cancel_observed || cancellation_requested()` 且归约结果不是 `Cancelled` 时，改判
`Cancelled`（`status=aborted` + `tool_cancelled`）并留一条 warn 诊断；未取消的轮次逐字段不变。

**5) 门禁**

- `src-tauri/tests/tauri_command_surface.rs`：`#[tauri::command]` 声明集合 ⊇
  `generate_handler![…]` 注册集合，且三条 ACP 命令两侧都在册（漏注册只会在运行时以
  `command not found` 暴露，编译不报）。
- `crates/pony-agent-core/tests/acp_cancel_race_contract.rs`：R1 冻结"取消后 agent 仍回
  `end_turn` ⇒ aborted"，并用 `duration_ms ≥ agent 收尾前的剩余延迟` 反证**确实走到了 Ok 分支**
  （否则"入口即取消"的旁路会让用例假绿）；R2 钉住未取消轮次仍是 `ok`。
- `acp::host` 单元测试：env 值不外泄、内置项判定、入参校验先于登记、相对 cwd 拒绝、重复 id。

## Alternatives considered

- **在 Tauri 层直接 `new session` / `prompt`**：文件最少，但把协议编排复制进 `src-tauri`，
  §4 P2 的"薄封装"约束与 Rust/前端职责分离不变量都被破。拒绝。
- **`acp_delegate_start` 真正异步（后台线程 + 结果轮询命令）**：需要第四个命令（`_status`）
  或前端轮询约定，超出 B163 的三命令范围；本轮用"调用方分配 id + 阻塞到本轮结束 +
  并发 `cancel`"覆盖"发起 + 取消"，流式观测走 `acp://delegate` 事件。若后续要长任务面板再补。
- **把 Win32 Job 调用复制一份到 `acp/`**：能绕开可见性问题，但同一套 Job 生命周期会有两份
  实现（本项目反复清理的"双真值"），故选择提升可见性 + 窄封装 `ProcessTree`。
- **复用 `plugin_host::capability::resolve_workspace_dir` 校验 cwd**：能被复用，但它的错误
  文案是插件口径（`plugin_workspace_invalid`），与 ACP 的 `acp_*` 错误码混用会误导调用方；
  改为 host 内做同一组校验（绝对路径 / 目录 / canonicalize），错误码 `acp_delegate_invalid_cwd`。
- **B169 只靠 dispatcher 兜住、不修会话层**：现状即如此，代价是"取消的一轮在界面上是成功"，
  契约面撒谎；会话层是唯一能判"这一轮被取消过"的地方，故在此收口。
- **Windows 纳管失败时降级为 `child.kill()` 继续启动**：看起来更"可用"，但与
  `agent::process` 既有的 fail-closed 口径（PA-077）相反，且会让"孙进程残留"从已知局限
  变成静默行为。拒绝。
- **让 `acp_delegate_start` 声明 `fs` / `terminal` 能力**：宿主桥没有授权清单与进程管理器
  上下文，声明即"声明了却没装"；本批不声明，权限请求一律 fail-closed `cancelled`。

## 对抗审查收据（2026-11-25）

- **antigravity ACP 子智能体**（session `52a2d7dc-9d4d-49fa-bfa1-1d4e4c2f1d01`，end_turn）提出 3 项
  阻断 + 若干建议，全部按最小修法处理：
  ① `AcpTransport::close()` 在宽限期发现直接子进程已退出时提前 `return`，跳过整棵树终止与
  Job 句柄释放 → 抽出 `terminate_tree()` 并在该分支调用（Unix 组 SIGKILL + Windows Job 终止/关闭）；
  ② Windows 未以 `CREATE_SUSPENDED` 创建子进程（`spawn → 纳管` 逃逸窗）+ `reap_child_and_tree`
  先 `wait` 后 close 导致 kill-on-close 兜底推迟 + Unix 启动失败路径未杀进程组 → 改为挂起启动 +
  纳管内 `resume_process`（复用 `agent::process` 的 ntdll 单点实现），回收统一为
  「组 SIGKILL → Job terminate/close → child.kill/wait」；
  ③ `acp::host` 单测断言"全局登记表为空"，并行执行下假红 → 改为只断言本用例自己的 id。
- **opencode ACP 子智能体不可用**：两次尝试均返回 `-32603 Free usage limit reached`
  （配额耗尽，非代码问题）。第二路由**独立子智能体**（fresh context，只读）替代，结论「未发现阻断项」，
  另指出两处低危并已修：`close()` 在 `terminated` 锁中毒时早退（改为 poison-into-inner，不跳过回收）、
  `cancel_delegation` 在锁外翻标志存在跨代竞态（改为锁内 `cancel()`，令牌翻标志不阻塞）。
- 未采纳的一条建议：把 B169 的归约条件收窄为只看 `cancel_observed`。理由：令牌只在本次委派
  登记期内可取消（`host::cancel_delegation` 的语义面），用户按下取消即应显示 `cancelled`；
  收窄会放过"取消发生在最后一次轮询之后、agent 刚好收尾"的真取消。R2 用例已钉住正常轮不受影响。

## Consequences

- 桌面端可列出外部 agent、发起委派并取消；委派期间事件走独立通道 `acp://delegate`，
  不会混进当前回合时间线。
- 委派面**不下放** `fs` / `terminal` 能力（与 `ponycli acp` 当前口径一致），需要这些能力的
  agent 仍走模型侧 `acp_delegate`（B175 装配面）。
- Windows 上 ACP 子进程随 `close()` / `Drop` 被 Job 终止；插件传输层与钩子命令路径仍是
  "只杀直接子进程"（本批未接线，文档注释已同步为该事实）。
- 取消竞态下终局一律 `aborted` + `tool_cancelled`；正常轮次逐字段不变（r2 钉住）。
- **已知限制（明示）**：Windows 分支在本机（Linux，且无 `x86_64-pc-windows-msvc` target）
  无法编译验证，判别依赖 ① 全部 Win32 调用仍只在 `windows_job.rs` 单点 ② Windows CI lane；
  `ProcessTree` 的 Windows 单元测试只在 Windows 上编译。
