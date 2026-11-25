# Agent Note: ACP P2 宿主面（Tauri 三命令）+ Windows 子进程树纳管

Status: implemented

## Problem

三件 ACP 遗留，收口前都是"契约不成立但被别的层兜住"：

- `backlog:B163`（P3）：ACP 只有 `ponycli acp` 一种宿主形态。桌面端既列不出可用外部 agent，
  也没法自己发起/取消一次委派——`acp_delegate` 是**模型侧**工具调用，界面没有入口。
  §4 P2 表把这三条 Tauri 命令标成"可选"，因此一直未落地。
- `backlog:B165`（P3）：`AcpTransport::spawn` 在 Unix 用 `process_group(0)` + 进程组 SIGKILL
  覆盖 agent 派生的孙进程；Windows 侧 `close()` 只 `child.kill()`，孙进程残留到自然结束。
  根因是 `agent/process/windows_job.rs` 的 `Job` 被收紧成 `pub(super)`，ACP 侧拿不到。
本批原本还含 `backlog:B169`（cancel-race 归约改判 aborted）。**该方向已在 2026-11-23 被
`architecture/2026-11-23-acp-cancel-race-ok-precedence.md` 否决并登记为 Ok 优先**（取消不覆盖
已到达的终态，取消的真正收口在宿主 dispatcher 停止链）。本批按"后登记的决策胜出"撤回该改动，
未把它带进主干；下节 Alternatives 记录这条取舍。

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

**4) 门禁**

- `src-tauri/tests/tauri_command_surface.rs`：`#[tauri::command]` 声明集合 ⊇
  `generate_handler![…]` 注册集合，且三条 ACP 命令两侧都在册（漏注册只会在运行时以
  `command not found` 暴露，编译不报）。
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
- **在 `prompt` 的 Ok 分支按取消事实改判 aborted（B169 原方向）**：本批先按此实现并写了
  R1/R2 冻结用例，但合并前发现 `architecture/2026-11-23-acp-cancel-race-ok-precedence.md`
  已把相反方向登记为决策（Ok 优先：Ok 表示这一轮事实已完成，用本地意图否定上游事实会丢弃
  已产出的正文，且与 dispatcher 的停止链职责重叠）。**决策冲突按"已合并的后登记决策胜出"处理**：
  撤回本批的会话层改动与用例，主干保持 Ok 优先。
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
- 合并冲突复核（2026-11-25）：`session.rs` 的 Ok 分支与 main 的 B169 契约相反，按上述
  "后登记决策胜出"取 main 版本；`src-tauri/src/lib.rs` 与 main 的 `acp_list_agents`
  （PR #54）重名，合并时只保留本批的**摘要面**版本（`AcpAgentSummary`，只回 env 键名），
  main 版的逐字段 `AcpAgentSpec` 版本删除，前端 `acp-api.ts` 同步改摘要类型。

## 合并后二轮审查（2026-11-25，基线 origin/main 7cdbcd33）

- **antigravity ACP 子智能体**（session `4b4340d2-fdf3-42db-bc91-489ff42ab2ea`）报 5 项阻断，全部已修：
  ① 坏提交（`generate_handler![…]` 里合并残留重复 `acp_list_agents` 且缺逗号 → 编译失败）——已修并 amend；
  ② `npm run size:check` 红：`AcpTransport::spawn` 221 行 > 200 —— 抽出 `prepare_child_command`
  （Command 构造 + env 合并）后降到 176 行，`size:check` 0 new violation；
  ③ 提交信息缺 `[decl-change-approved]`（本批改了 `docs/observability.yaml`）——已补；
  ④ 三条在册 clippy lint 会破基线：`tauri_adapter` 的 `unreachable_pub`（改 `pub(crate)`）、
  `acp_delegate_cancel` 的 `needless_pass_by_value`（Tauri 命令参数必须 `DeserializeOwned`，
  带理由 `#[allow]`）、`host.rs` 的 `redundant_clone`（改移动语义）；
  ⑤ 文档口径与已合并的 B169「Ok 优先」契约矛盾（`host.rs` 两处 + `lib.rs` 一处）——改为
  「取消命中且 agent 未产出终态 ⇒ aborted；agent 已同步回 Ok 终态 ⇒ 保留该终态」。
- 专项核查（antigravity 结论）：Windows 纳管失败回滚与句柄生命周期无阻断（无 double-close /
  泄漏 / 逃逸窗）；`host.rs` 取消令牌登记与注销无泄漏或跨代竞态；Tauri 三命令无协议逻辑泄漏。
- **clippy 门禁本机不可用（环境缺陷，非本改动）**：`rustc`(/usr/bin/rustc-1.91) 与
  `clippy-driver`(/usr/lib/rust-1.91) 非同源构建，`npm run rust:lint` 报 1113 条
  `E0514 found crate … compiled by an incompatible version of rustc`；清空 `target-check` 重跑、
  全新 `--target-dir` 重跑均复现，且**在纯净 `origin/main` 的独立 worktree 上同样复现 1112 条**。
  ⇒ 判据归 CI（GitHub runner 工具链同源），本批不据此判红绿，但上述三条 lint 已静态修掉。

### 复审第二轮（antigravity session `464b284d`，HEAD a431eb9e）

逐条核实上一轮 5 项阻断：4 项已修好，第 4 项修法引入新违例；另由 `prepare_child_command` 抽取引入 1 条。
两条新增阻断均已修：

- `#[allow(clippy::needless_pass_by_value)]` 缺 `reason` ⇒ 违反在册 `allow_attributes` /
  `allow_attributes_without_reason`：改为 `#[expect(clippy::needless_pass_by_value, reason = "…")]`
  （仓内既有写法；Tauri 命令参数必须 `DeserializeOwned`，`&str` 不成立）。
- `prepare_child_command` 内 `workspace` 已改为 `&Path`，遗留 `.current_dir(&workspace)`
  （`&&Path`）⇒ `clippy::needless_borrows_for_generic_args`：改为 `.current_dir(workspace)`。

审查路径记录（诚实留痕）：`opencode` 两轮均返回 `-32603 Free usage limit reached`（配额耗尽，非代码问题），
第二路尝试由独立 fresh-context 子智能体承担，但超出单轮时限（120s）未产出结论；故本轮"两路"实际为
**antigravity 两轮独立复审（6bd8c89d → a431eb9e，后者逐条核实修复）**，opencode 一路缺位已明示。

## Consequences

- 桌面端可列出外部 agent、发起委派并取消；委派期间事件走独立通道 `acp://delegate`，
  不会混进当前回合时间线。
- 委派面**不下放** `fs` / `terminal` 能力（与 `ponycli acp` 当前口径一致），需要这些能力的
  agent 仍走模型侧 `acp_delegate`（B175 装配面）。
- Windows 上 ACP 子进程随 `close()` / `Drop` 被 Job 终止；插件传输层与钩子命令路径仍是
  "只杀直接子进程"（本批未接线，文档注释已同步为该事实）。
- `acp_list_agents` 的返回从"清单原样"改为"摘要（只回 env 键名）"：env 值不再跨 IPC 进渲染进程。
- cancel-race 的归约优先级维持 main 的 Ok 优先（本批未改动会话层）。
- **已知限制（明示）**：Windows 分支在本机（Linux，且无 `x86_64-pc-windows-msvc` target）
  无法编译验证，判别依赖 ① 全部 Win32 调用仍只在 `windows_job.rs` 单点 ② Windows CI lane；
  `ProcessTree` 的 Windows 单元测试只在 Windows 上编译。
