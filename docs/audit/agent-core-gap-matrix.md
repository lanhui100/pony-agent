# pony-agent Agent 核心能力差距矩阵

> 基线：claude-code `2.1.88`（sourcemap 还原源码 `/home/dm/claude-code-sourcemap/restored-src/src`）、DSH（`/home/dm/deepseek-harness/packages`）
> 审计对象：`/home/dm/pony-agent`（`crates/pony-agent-core/src` + `src-tauri/src` + `src`）
> 方法：22 条能力轴 × 3 个独立只读审计员（分区互斥，均给出 `path:line`）+ Lead 抽验 + 独立反证核验 subagent
> 原始报告：[claude-code.md](raw/claude-code.md) · [dsh.md](raw/dsh.md) · [pony-agent.md](raw/pony-agent.md) · [verify-gaps.md](raw/verify-gaps.md)
> 审计日期：2026-10-17｜纯静态只读审计，未编译/未运行

## 一句话结论

pony-agent **没有"绝对空白"的能力域（0 项 ABSENT）**：22 轴全部有对应模块。真正的差距是 **19 项 PARTIAL 中的"有壳无芯"**——接口/trait/工具名齐备但核心实现缺失或未接线（4 处最硬：并发工具调用被硬拒、subagent/workflow 是桩、MCP 只有 FakeTransport、LSP 是字符串匹配）。与两个基线相比，最大的**结构性代差**集中在：回合引擎并发化（A01）、真实多代理委派（A08）、MCP 生态接入（A13）、上下文工程（A09）、可编程扩展面（A07/A14/A17/A22）。

## 总账

| 库 | HAS | PARTIAL | ABSENT |
|---|---|---|---|
| claude-code 2.1.88 | 20 | 2（A05 无 PTY、A18 无工作流引擎） | 0 |
| DSH | 17 | 5（A06 A10 A15 A16 A19） | 0 |
| **pony-agent** | **5**（A02 A03 A06 A11 A15） | **17** | **0** |

## 三方矩阵（22 轴）

| # | 能力轴 | claude-code | DSH | pony-agent | 差距判定 |
|---|---|---|---|---|---|
| A01 | 主循环/回合引擎 | HAS 流式 loop+工具边流边跑 | HAS turn/step+并行工具池 | PARTIAL 单步单工具，多工具直接 `Err` | **P0** |
| A02 | 工具系统框架 | HAS zod+deferred+落盘 | HAS registry+schema+spill | HAS 注册表+schema+Deferred+tool_search | 持平，领先 DSH（有 tool_search） |
| A03 | 文件与编辑 | HAS（无 apply_patch） | HAS（无 apply_patch） | HAS read/write/edit/apply_patch+SHA256 CAS | **领先** |
| A04 | 代码检索 | HAS ripgrep+LSP 符号 | HAS ripgrep | PARTIAL 正则/glob，无符号导航 | P2（随 A19） |
| A05 | Shell 执行 | PARTIAL 无 PTY | HAS 前后台+PTY | PARTIAL 无 PTY，输出轮询拉取 | P2 |
| A06 | 权限与沙箱 | HAS 6 模式+allow/deny+OS 沙箱+域名白名单 | PARTIAL 无网络管控/无规则名单 | HAS 路径分区+沙箱矩阵+出口白名单 | 领先 DSH；缺工具级规则名单（P1） |
| A07 | Hooks | HAS 27 事件×4 执行体 | HAS CC/Codex shell 钩子桥 | PARTIAL 钩子点齐备，执行器仅 Noop | **P1** |
| A08 | 子代理与团队 | HAS fork/fresh+团队板+多后端 | HAS spawn/fork+5 后端+workflow DAG | PARTIAL 团队板真实，subagent/workflow 是桩 | **P0** |
| A09 | 上下文压缩 | HAS auto/micro/api-compact | HAS 压缩+裁剪+spill+图片 offload | PARTIAL 阈值压缩+摘要，无 spill/microcompact | **P0/P1** |
| A10 | 记忆与项目指令 | HAS 四层+memdir+团队记忆 | PARTIAL 分层指令，无跨会话记忆 | PARTIAL 单宪法文件热加载，无分层/召回 | P2 |
| A11 | 会话持久化 | HAS transcript+resume+fork+**文件级 rewind** | HAS append-only+checkpoint+FTS（无 rewind） | HAS SQLite+fork/branch/checkout（仅对话层） | P1（缺文件级回滚） |
| A12 | 计划模式 | HAS 模式切换+审批门禁 | HAS /plan+exit_plan_mode 批准 | PARTIAL 状态机+审批，无只读门禁 | P1 |
| A13 | MCP 集成 | HAS stdio/SSE/HTTP+OAuth+resources | HAS stdio/http+重连 | PARTIAL 有 stdio 子进程传输，无协议握手/SSE/HTTP | **P0** |
| A14 | Skill/扩展 | HAS skill 渐进+插件市场+101 命令 | HAS 全插件 Cordis | PARTIAL 扫描 SKILL.md，无插件宿主 | P2 |
| A15 | Provider 路由 | HAS 4 类（同协议族） | PARTIAL 无跨 provider 降级 | HAS 多协议+SSE+reasoning+退避+cache | **领先** |
| A16 | 成本与用量 | HAS 分模型计价+`maxBudgetUsd` 熔断 | PARTIAL 只记账无上限 | PARTIAL token/预算台账，无金额与熔断 | P2 |
| A17 | 可观测性 | HAS OTel span+事件 sink | HAS OTLP 双通道+不变量 | PARTIAL 自建 trace，无 OTel/OTLP | P1 |
| A18 | 后台任务与调度 | PARTIAL 有 cron，无工作流引擎 | HAS jobs+cron+workflow 引擎 | PARTIAL jobs 有，无 cron、workflow 为桩 | P2（workflow 归 A08） |
| A19 | 代码智能/LSP | HAS 客户端+诊断回流 | PARTIAL 四只读操作 | PARTIAL 工具名在，实为字符串启发式 | **P1** |
| A20 | 富媒体与外部 | HAS 图像/PDF+Chrome+computer-use | HAS 图像+web+browser/computer 插槽 | PARTIAL 图像+文档+web，无浏览器/computer | P1 |
| A21 | 安全防御 | HAS 隐写清洗+注入检测+密钥扫描 | HAS 凭据引用+审计+HMAC | PARTIAL 密钥库+脱敏+危险命令拦截 | P2 |
| A22 | 交付与集成面 | HAS SDK+headless+IDE 桥 | HAS SDK/ACP/webhook/API 网关 | PARTIAL CLI/TUI/headless，无 SDK/ACP | P1 |

## P0：能力断层（直接限制 agent 上限）

### 1. A01 回合引擎不支持并发工具调用（最明确的代差）

- **基线**：claude-code `services/tools/StreamingToolExecutor.ts:40`（token 未完成即按 `isConcurrencySafe` 分区并发跑工具，中断保留已完成结果）、`query.ts:659`；DSH `packages/core/agent-loop/src/tool-calls.ts:90`（parallel 池）+ `assistant-stream.ts:122`（中断保留已产出块）。
- **现状（核验后精确表述）**：provider 层不支持多工具调用——Anthropic 返回多个即 `Err`（`crates/pony-agent-core/src/agent/provider/mod.rs:2249`，非流式同 `:2938`），OpenAI 路径**静默丢弃多余调用只取首个**（`provider/openai_sse.rs:105-108`），决策结构为单值 `tool_call: Option<ToolCall>`（`provider/mod.rs:225`），执行侧 `runtime/turn_runner.rs:189,201` 也按单调用跑。注：治理执行器本身是并发的（`child_dispatch.rs:249` `std::thread::scope`，上限 24，`tools.rs:160`），但那只发生在**单个工具内部**的 fan-out，且流必须先结束。全库无「边流边执行」证据（`grep -niE "stream.*tool_exec|early.*execut"` 零命中）。
- **影响**：单轮只能串行一次一个工具，"并行读 5 个文件/并行 grep"这类高频范式退化为多轮往返 → 墙钟与 token 双输；也无法实现"边流边执行"。
- **建议落点**：`provider/mod.rs` 决策结构改 `Vec<ToolCall>` + `runtime/tool_exec.rs` 加并发池（按 descriptor 已有 `concurrent_safe` 字段 `tools.rs:375` 分区，串行不安全工具）；`turn_stream.rs` 增流式增量派发钩子。

### 2. A08 子代理与并行委派是桩，无独立上下文子代理

- **基线**：claude-code `tools/AgentTool/AgentTool.tsx:325`（fork/fresh）、`utils/swarm/backends/types.ts:9`（tmux/iTerm/in-process 三后端）、`tools/TaskUpdateTool/TaskUpdateTool.ts:313`（blockedBy）；DSH `packages/subagent/tool-subagent/src/index.ts:380`、`subagent-fork-in-process/src/index.ts:23`（fork 以父已完成 turn 为 seed）、`packages/workflow/tool-workflow/src/index.ts:275`（agent/parallel/pipeline 脚本编排）。
- **现状**：`agent/orchestration.rs:200`（`subagent` 仅拼 `"Subagent completed execution for prompt: …"`）、`:223`（`workflow` 仅回显 step id）、`:236`（`spawn_teammate` 登记真实）；`tools.rs:1212` 直连桩；`SubagentArgs.fork_context` 未生效。真实资产只有工具级 `child_dispatch.rs:108`（lineage/预算/取消）与 `graph.rs:307`（回合决策状态机，不是通用 DAG）。
- **影响**：无法"分而治之"——大任务不能拆给独立上下文的子代理并行，审计/多视角评审/大规模扫频这类范式缺失；`workflow` 名字在但无引擎。
- **⚠️ 比缺失更危险（G-02，W25 实测已修正）**：这 10 个工具的 **handler 仍返回 `status:"ok"` 假回执**（合成字符串 + 进程内静态表），模型若按名字硬调就会收到"已完成"回执，实际什么都没发生。注意：生产工具面投影（`builtin_turn_tool_contract_views()` / `anthropic_tools_payload`）**当前已不含**这 10 个名字（真实面 21 个），而 `provider/mod.rs:4665` 附近的内联断言仍按"含 10 个假工具"写死，**基线本就 FAILED**——即审计初稿"被注册为 model_visible"只对陈旧断言成立，对运行时 payload 不成立。
- **建议落点**：新增 `agent/subagent/` 子运行时（独立 session + 独立上下文 + 结果回传），复用现有 `budget.rs` 共享台账与 `child_dispatch.rs` lineage 约束；`orchestration.rs` 桩函数替换为真实派生，**在真实实现落地前先把这些工具从 model_visible 面摘除或标注为不可用**；补 DAG 波次执行器（可参考 `graph.rs` 状态机扩展）。

### 3. A13 有传输无客户端：MCP 生态接不进来

- **基线**：claude-code `services/mcp/client.ts:12/9/14`（Stdio/SSE/StreamableHTTP 三传输）+ `services/mcp/auth.ts:1293`（OAuth）+ resources/prompts 工具；DSH `packages/mcp/mcp-client/src/index.ts:54/82` + `connection.ts:28`（重连退避）。
- **现状（核验后已收窄，见 G-03）**：**有传输、无客户端**。`agent/tool_runtime.rs:395` 确有真实 `StdioMcpTransport`（`Command::new` + stdin/stdout 单行 JSON-RPC，`:411-460`，有集成测试 `tests/hotloader_and_mcp_contract.rs:25`）；缺口是：① 无 `initialize` / `tools/list` 握手，method 名是自造的 `list_resources`/`read_resource`（`tool_runtime.rs:452-458`），不符合 MCP spec；② 每次请求 spawn 一个进程、无长连接与能力协商；③ 无 SSE/HTTP/streamable 传输；④ `McpResourceSurface` 未接线，`mcp_resource_read` 当前 fail-closed 返回 `source_unavailable`（`runtime/tool_exec.rs:327-333`，注释自认 "Wiring … is follow-up work"）。另有 `FakeMcpTransport`（`tool_runtime.rs:359`）仅用于测试。
- **影响**：真实 MCP 服务器（标准握手）接不进来，`mcp_resource_*` 工具对模型呈现为"有但永远失败"，属假接线。
- **建议落点**：`tool_runtime.rs` 补 `initialize`/`tools/list` 握手 + 复用长连接会话，新增 `HttpMcpTransport`（reqwest + SSE）；把 `McpResourceSurface` 接到 `dispatcher`/`tool_exec` 正常路径（去掉 fail-closed）；重连退避复用 `retry.rs:38`。

### 4. A09 上下文工程缺 spill 与 microcompact，长任务会顶到窗口

- **基线**：claude-code `services/compact/autoCompact.ts:241`、`query.ts:414`（microcompact）、`services/compact/apiMicrocompact.ts:64`、`utils/toolResultStorage.ts:268`（大结果落盘）；DSH `packages/spill/spill-local/src/index.ts:149` + `spill-policy/src/index.ts:113`（按 token 计价的通知）、`compaction-tool-result-pruner/src/index.ts:83`。
- **现状**：`runtime/turn_prep.rs:39` 阈值压缩 + `compression.rs:146` XML 摘要 + `context.rs:1299` 按 token 裁剪；大输出仅按字节截断（`tools.rs:158` `MAX_FULL_READ_BYTES=120_000`）→ 截断即丢失，无落盘回读。
- **影响**：长会话/大仓库任务里"看不全又记不住"，且截断销毁证据；对比基线是"落下再按需回读"。
- **建议落点**：`tools.rs` 结果截断处改为写 spill 文件 + 返回引用（参考 DSH 的 token 计价通知），新增 `spill` 模块；`turn_prep.rs` 增加"工具结果按 token 剪枝"预处理（microcompact 的低成本子集）。

## P1：明显落后（影响可用性与工程化）

| # | 缺口 | 基线证据 | pony-agent 现状 | 建议落点 |
|---|---|---|---|---|
| 5 | A07 外部命令 Hooks 执行器为空 | CC `services/tools/toolHooks.ts:39`、`schemas/hooks.ts:33`（command/prompt/http/agent 四体）；DSH `packages/hooks/hook-protocol/src/events.ts:76` | 19+ 钩子点与注册表齐备，仅 `NoopHookExecutor`（`hooks.rs:982`），无 settings 加载与 shell 钩子 | `hooks.rs` 实现 `CommandHookExecutor`（进程 + JSON stdin/out + 超时/阻断语义），加载 `.claude/hooks.json` 同构配置 |
| 6 | A19 LSP 是文本启发式 | CC `services/lsp/LSPClient.ts:1`、`LSPDiagnosticRegistry.ts:193`；DSH `packages/lsp/tool-lsp/src/index.ts:110` | `lsp.rs:132` 用 `"fn {} "` 字符串匹配，`lsp.rs:205` diagnostics 恒空 | `lsp.rs` 接 `tower-lsp`/`lsp-server`，spawn server + JSON-RPC；诊断注入 `context.rs` 附件；A04 符号导航随之解锁 |
| 7 | A11 缺文件级 checkpoint/rewind | CC `utils/fileHistory.ts:347`（`fileHistoryRewind`）与对话 rewind 双轨 | 仅对话层节点回滚（`session/store.rs:1364/1501/1003`），无工作区快照 | 写工具在写前登记文件快照（复用 CAS 的 hash，`tools.rs:1703`），`session` 增 `WorkspaceSnapshot` 类型 |
| 8 | A12 无只读 plan 模式门禁 | CC `EnterPlanModeTool.ts:92` + `permissionSetup.ts:608`（模式切换影响权限）；DSH `packages/plan/plan-mode/src/index.ts:67` | `plan_state.rs:177` 状态机 + host 审批，但 Draft→Executing 自动推进（`:326`），无"未批准禁止写"拦截；todo 无 `plan_id` | `governed_executor.rs` 增加 plan 相位闸门（写类工具在 Draft 态直接拒），`todo.rs` 关联 plan step |
| 9 | A20 无浏览器/computer use | CC `utils/claudeInChrome/mcpServer.ts:248`、`utils/computerUse/executor.ts:259`；DSH `packages/browser-use`、`packages/computer-use` | `image_artifact.rs`/`document_conversion.rs`/`web_access.rs` 有，browser/cdp/computer 全库 0 命中 | 新增 `browser` 工具族（CDP over Chrome / playwright 子进程），经 `dispatcher.rs` 注册；DOM 快照 + 截图回传 |
| 10 | A06 缺工具级 allow/deny 规则与审批三态 | CC `types/permissions.ts:16`（acceptEdits/bypass/dontAsk/plan）+ `permissions.ts:213`；DSH `user-approval/src/index.ts:67` | 有 `ToolPolicyEvaluator`+`PermissionVerdict` 模型（`dispatcher.rs:113-145`）、路径分区（`path_permission.rs:415`）、host 审批；但 `deny_tools`/`disallowed_tools` 0 命中，且默认 `LegacyCompatiblePolicyEvaluator` 除 Ask 外一律 `Allow`（`governed_executor.rs:66-95,178`，注释自认待 permission-contract 里程碑） | `app_settings.rs` + `governed_executor.rs` 给出可配置 rules 段（工具名/参数正则 + allow/ask/deny），替换兼容层 |
| 11 | A17 无 OTel/OTLP | CC `utils/telemetry/sessionTracing.ts:216`；DSH `packages/telemetry/otel/src/index.ts:22` | `telemetry.rs:27` 自建 trace + `trace_persistence.rs:44` 落盘 + `ponysentry` 私有上报，`otlp`/`opentelemetry` 0 命中 | 接 `opentelemetry-otlp`，把现有 `TurnTraceStep` 映射为 span（含 trace_id），保留私有上报做双写 |
| 12 | A22 无对外 SDK/ACP/webhook | CC `entrypoints/agentSdkTypes.ts:112`、`cli/print.ts:594`；DSH `packages/sdk/server/src/server.ts:75`、`packages/acp/acp/src/index.ts:379` | CLI/TUI/headless harness 三入口 + Tauri IPC（`src-tauri/src/lib.rs:49`），无 axum/tonic/SDK 包 | 先做 ACP 或 HTTP+SSE 服务（复用 `control_plane` 10 个命令模块），代价低收益高（IDE 集成） |

## P2：可后置（非阻断，但基线有）

| # | 缺口 | 现状 | 备注 |
|---|---|---|---|
| 13 | A16 无金额计价与预算熔断 | `budget.rs:83` 只记调用/字节/时限；`/cost` 是占位（`bin/cli.rs:516`） | 若走 polyllm 计费，拿返回的 usage 计价即可；claude-code `QueryEngine.ts:972` 用 `maxBudgetUsd` 直接熔断 |
| 14 | A18 无 cron/定时调度 | `jobs.rs` 有 start/output/kill/list，`cron`/`scheduler` 0 真实命中 | 与 A08 workflow 一起做更划算 |
| 15 | A05 无 PTY | `terminal.rs:381` 为管道 + 轮询（但 `terminal_open/send/read/signal/close` 六件套真实完备：2MB 环形缓冲、16 会话、Windows Job Object/POSIX 进程组收容、有黑盒验收 `tests/pa104_terminal_acceptance.rs`）；claude-code 同样缺 PTY，DSH 有 | 真实缺口仅「无伪终端」：全屏 TUI/密码提示/ANSI 光标类程序不等价 |
| 16 | A10 记忆无分层与跨会话召回 | `workspace_hotloader.rs:97` 只取首个宪法文件；`LongTermMemoryRecord` 是否存在跨会话注入 `[UNVERIFIED]` | claude-code 四层 CLAUDE.md + memdir |
| 17 | A14 无插件宿主、无正文按需加载 | `workspace_hotloader.rs:141` 扫 SKILL.md，`context.rs:555` 仅注入名称/描述 | 与 A07 命令钩子一起补才闭环 |
| 18 | A21 无提示注入检测、无独立审计日志 | `sanitizer.rs` 只脱敏密钥；审计是内嵌摘要（`session/types.ts:579`） | claude-code 有 Unicode 隐写清洗 + bash 注入检测 + 权限决策日志 |
| 19 | A04 无符号级导航 | `search.rs:141` 正则 + `git_search.rs:65` 模糊 | 依赖 A19 真实 LSP，不单独做 |

## pony-agent 已经不落后甚至领先的部分（避免误判全盘落后）

1. **A03 文件编辑**：`apply_patch` + SHA256 CAS（`tools.rs:1703`、`patch.rs:131`），两个基线都没有 apply_patch。
2. **A15 Provider**：多协议 + SSE + reasoning + 抖动退避 + prompt cache 映射（`provider/mod.rs:364/448`、`retry.rs:38`），比 DSH（无跨 provider 降级）与 claude-code（只有 Anthropic 协议族）更宽。
3. **A02 工具框架**：`ToolIdentity`/`ToolExposure(Deferred)`/`ToolExecutionPolicy` 三层描述符契约 + tool_search 动态曝光（`tools.rs:334/365/375`、`tool_search_elevation.rs:76`），DSH 反而没有 tool_search。
4. **A06 沙箱**：三重后端矩阵 + fail-closed + Windows Job Object（`sandbox.rs:66/175/245`、`process/windows_job.rs`），比 DSH（无网络出口管控）更完整。
5. **A11 会话**：SQLite + fork/branch/checkout 回滚 + 崩溃恢复（`session/store.rs:1364/1501`），DSH 无 rewind。
6. **KV-cache 保护型热加载**：宪法/skills 指纹变更产出 append-only delta（`workspace_hotloader.rs:67/76`），是为 prompt cache 友好设计的差异化资产。
7. **多宿主控制面**：同一 `HostControlPlane` 驱动 Tauri/CLI/TUI/headless 四宿主（`control_plane/mod.rs`，`src-tauri/src/tauri_adapter.rs:101`）。

## 反向差距：基线独有、pony-agent 完全没有的"杀手能力"

| 能力 | 出自 | 为什么值得抄 |
|---|---|---|
| 流式工具并发执行器（边流边跑） | claude-code `StreamingToolExecutor.ts:40` | 直接决定同轮吞吐，体验差距最直观 |
| 文件级 rewind 到任意历史消息 | claude-code `utils/fileHistory.ts:347` | 让 agent 敢试错，回滚成本从"人工 git"降到一次命令 |
| PTC（工具 schema 折叠成 `run_code` SDK） | DSH `core/tools/src/ptc.ts:30/333` | 省大量工具 schema token，工具越多收益越大 |
| 按 token 计价的大输出 spill 通知 | DSH `spill/spill-policy/src/index.ts:113` | 让"看不完"变成"按需回读"而不是"被截断" |
| 双协议 hooks 桥（CC + Codex 线协议） | DSH `hooks/hook-claude-code`、`hooks/hooks-codex` | 直接吃现有钩子生态，免设计 |
| 多形态子代理后端（in-process/ACP/SDK/Codex） | DSH `subagent/*` | 子代理可跨进程/跨产品复用 |
| 全插件 Cordis + 运行时 API 自省 | DSH `extensions/tool-cordis/src/index.ts:23` | 扩展面从"改代码"变成"装插件" |

## 独立反证核验（L3 只读，15 条缺口逐条回查）

统计：**CONFIRMED 13 / REFUTED 1（部分子句）/ UNVERIFIED 0**。核验纠正了 3 处过强表述（已回写本文档），并追加 7 条发现（详见 [verify-gaps.md](raw/verify-gaps.md)）。

| 编号 | 结论 | 核验要点 |
|---|---|---|
| C01 并发工具 | CONFIRMED | provider 层单调用模型（`provider/mod.rs:225`）+ Anthropic 报错（`:2249/:2938`）+ OpenAI 静默丢首（`openai_sse.rs:105-108`）；不得表述为"无并发执行器"（`child_dispatch.rs:249` 有单工具内 fan-out） |
| C02 子代理桩 | CONFIRMED | `orchestration.rs:200/223` 合成字符串；`spawn_subagent|child_session|session_fork` 全库 0 命中 |
| C03 MCP | **REFUTED（部分）** | `tool_runtime.rs:395` `StdioMcpTransport` 真实可用（有集成测试）；准确缺口是"有传输、无客户端" |
| C04 Hooks | CONFIRMED | 三个 Noop executors 即全部生产实现，默认接线 `runtime/mod.rs:177` |
| C05 LSP | CONFIRMED | 213 行文件、`line.contains` 匹配、diagnostics 恒空、连 `lsp` 依赖都没有 |
| C06 输出溢出 | CONFIRMED | 超预算 `output_budget_exceeded` 硬失败（`dispatcher.rs:1099/1434`），无 spill/回读 |
| C07 文件回滚 | CONFIRMED | `workspace_rollback_capable` 恒 false（`hooks.rs:240/1773`、`session/store.rs:3826`） |
| C08 浏览器 | CONFIRMED | Rust/TS 双侧 `playwright|puppeteer|cdp|webdriver|chromium` 全 0 命中 |
| C09 工具权限规则 | CONFIRMED（附限定） | 有权限模型（`dispatcher.rs:113-145`）但生产兼容层全放行（`governed_executor.rs:66-95,178`） |
| C10 plan 门禁 | CONFIRMED | `plan_mode` 0 命中；Plan 仅是任务清单状态机 |
| C11 成本熔断 | CONFIRMED | `/cost` 空壳（`cli.rs:411` 承诺 vs `:516-521` 实际只打印费率名） |
| C12 OTel | CONFIRMED | 三份 Cargo.toml 均无依赖，`opentelemetry|otlp` 0 命中 |
| C13 cron | CONFIRMED | `cron|scheduler|定时` 全 0 命中 |
| C14 对外服务面 | CONFIRMED | 仅 Tauri IPC 86 命令，无 webhook/acp/grpc/axum |
| C15 PTY | CONFIRMED | `Stdio::piped` 非伪终端；`terminal_*` 功能本身真实完备 |

三条最值得单独立项（核验追加）：

1. **假能力暴露（G-02，W25 实测修正）**：工具面投影已排除这 10 个名字，但 handler 仍返回假 `ok` 回执，且内联断言陈旧导致**基线测试本就 FAILED**——危害形态是"模型按名硬调即被骗"，不是"每次都在工具列表里看到"。已在 W25 修正为 `capability_unavailable`。
2. **默认工具引擎已是治理 dispatcher（G-01）**：`runtime/builder.rs:78-84` 走 `build_governed_executor`，具备原子预算/嵌套深度/cycle 拒绝/并发子派发/沙箱 fail-closed——本项目的治理骨架其实是强项，别误判为缺失。
3. **防反向误判（G-06）**：`web_access`（含 host/端口 deny list）、anydoc 文档转换、`view_image`、`patch`、`git_search`、PonySentry、SQLite 会话、对话级 fork、`workspace_batch` 并行（≤24）都真实存在，本表未列为缺口。

## 复现与溯源

- 三方原始审计：`docs/audit/raw/{claude-code,dsh,pony-agent}.md`（22 轴逐条 `path:line`、检索关键词、覆盖盲区）
- 缺口反证核验：`docs/audit/raw/verify-gaps.md`（对上述 P0/P1 结论做同义词反证检索，判定 CONFIRMED/REFUTED/UNVERIFIED）
- 一致性抽验：`provider/mod.rs:2249`、`orchestration.rs:200`、`hooks.rs:982`、`lsp.rs:205`、`tool_runtime.rs:395`；基线侧 `core/agent-loop/src/agent.ts:296`、`tools/LSPTool/LSPTool.ts:473` 均已回放。

## 局限（不要把本表当精确质量分）

1. 纯静态阅读，未编译未运行；"接线是否真实生效"靠代码结构判断，个别行号可能对应当前 HEAD 的附近而非精确行。
2. 三个基线规模巨大（claude-code 1884 个 TS 源文件、DSH ~300 包、pony-agent 144 个 .rs），超大文件（如 `tools.rs` 12292 行、`control_plane/mod.rs` 9606 行）只做定点抽读。
3. PARTIAL 不等于"做得差"：pony-agent 多个 PARTIAL 轴在子维度上强于基线（见上节），本表只衡量"基线有而此处无"的部分。
4. 15 条关键缺口已经独立反证（13 CONFIRMED / 1 REFUTED / 0 UNVERIFIED），本文档结论已按核验结果修正；未核验项（如 A10 跨会话召回）以 `[UNVERIFIED]` 标注。
