# 反证核验报告（L3 独立核验 / 只读）

- 核验对象：`/home/dm/pony-agent`（`crates/pony-agent-core/src` Rust 核心 105 文件 / 123,141 行；`src-tauri/src`；`src` Vue/TS；`scripts`）
- 核验方式：每条结论 ≥2 组同义词/别名 `grep -rn`（排除 `node_modules` / `dist` / `target`），命中后 `sed -n` / `read` 局部窗口确认「是否真实接线」，并区分：真实实现 / 仅 trait·类型 / 仅测试 / 仅注释 / 未接线 / 内存桩
- 旁挂目录 `open-claude-ui/`、`codex-openai/`、`node_modules`、`dist`、`target`、`target-check`、`target-test` 未计入实现证据
- 本文件为唯一写入物，未修改任何源码。
- 判定口径：`CONFIRMED`＝欠缺成立；`REFUTED`＝存在真实实现（附 path:line）；`UNVERIFIED`＝证据不足

## 判定总表

| 编号 | 判定 | 反证/支撑证据 path:line | 用过的检索关键词 |
|---|---|---|---|
| C01 | **CONFIRMED** | ① `crates/pony-agent-core/src/agent/provider/mod.rs:2249`（Anthropic SSE 多 tool_use 直接 `Err("Anthropic 返回了多个工具调用，但当前运行时仅支持单工具调用。")`），同文案非流式分支 `provider/mod.rs:2938`；② OpenAI 路径静默丢弃多余调用，只取首个 `provider/openai_sse.rs:105-108`（`tool_calls.into_iter().next()`）；③ 全部 provider 结果型别只带单个调用 `provider/mod.rs:225`（`pub tool_call: Option<ToolCall>`）、`provider/openai_sse.rs:8`、`provider/responses_api.rs:176,733`；④ 运行时按单个 `Option<ToolCall>` 选择执行 `runtime/turn_runner.rs:189,201`；⑤ 全库无「边流边执行」：`grep -rniE "stream.*tool_exec|early.*execut|execute.*while.*stream"` 零命中。**限定**：`workspace_batch` 确实并发派发只读子调用（`agent/child_dispatch.rs:249` `std::thread::scope`，上限 `tools.rs:160` =24），但属「单工具内部 fan-out」，非 provider 级多工具调用，且在任何工具开始前流已结束 | `多个工具调用\|multi_tool\|parallel_tool_calls\|tool_calls`；`join_all\|join_set\|spawn_blocking\|rayon\|tokio::spawn\|concurren`；`stream.*tool_exec\|early.*execut\|execute.*while.*stream\|preempt.*tool` |
| C02 | **CONFIRMED** | 桩函数确认：`agent/orchestration.rs:200`（`subagent` 仅 `format!("Subagent completed execution for prompt: {}", args.prompt)`，`:206` 起 return）、`orchestration.rs:223`（`workflow` 仅把 `step.id` 收进 Vec）。更严重：`spawn_teammate` 只登记进程内静态表 `orchestration.rs:236`（`TEAMMATES`/`TEAMMATE_INBOXES` 静态 Mutex，`orchestration.rs:190-194`），`list_agents` `orchestration.rs:279`。全库无独立上下文/独立会话派生：`spawn_subagent\|derive_agent\|child_session\|session_fork\|sub_session\|delegate_agent\|agent_spawn` **零命中**。`child_dispatch.rs` 是「复合工具的子工具调用」（line 3-6 注释明示），非子代理。且这些桩被注册为 model_visible 工具（`provider/mod.rs:4645-4700`、`tools.rs:1188,1262`）→ 模型可调用但拿不到真实派生 | `spawn_subagent\|derive_agent\|child_session\|session_fork\|fork_session\|sub_session\|delegate_agent\|agent_spawn`；`orchestration::\|spawn_teammate\|TeamTaskItem\|teammate`；`subagent`（`--include=*.rs` 全库） |
| C03 | **REFUTED（部分子句）** | 声称「仅 FakeMcpTransport（`tool_runtime.rs:359`）」不成立：`agent/tool_runtime.rs:395` 存在 `StdioMcpTransport`，真实 `Command::new` 派生子进程 + `stdin/stdout` 管道 + 单行 JSON-RPC 往返（`:411-460`），并有黑盒集成测试 `crates/pony-agent-core/tests/hotloader_and_mcp_contract.rs:25-37`。**但**剩余缺口真实存在，见追加发现 G-03：无 SSE/HTTP 传输、无 `initialize`/`tools/list` 握手（method 名是自造 `list_resources`/`read_resource`，`tool_runtime.rs:452-458`）、且 `McpResourceSurface` 未接线（`runtime/tool_exec.rs:327-330` 明确 fail-closed，"Wiring … is follow-up work"） | `FakeMcpTransport\|McpTransport\|mcp_client\|McpClient\|jsonrpc\|json-rpc\|tools/list\|StdioTransport\|SseTransport\|streamable`；`"initialize"\|tools/list\|list_tools`；`McpResourceSurface\|StdioMcpTransport`（全库，含 tests） |
| C04 | **CONFIRMED** | 生产实现只有 Noop：`agent/hooks.rs:982` `pub struct NoopHookExecutor`，另有 `NoopMemoryWriteHookExecutor`（`:991`）、`NoopHistoryStateHookExecutor`（`:1000`），三者即 trait `AgentHookExecutor`（`:950`）的全部生产实现；默认接线 `runtime/mod.rs:177` `hook_executor: Box::new(NoopHookExecutor)`。无外部命令钩子：`hooks.rs` 内 `Command::new|std::process` **零命中**；`AgentHookDescriptor{` 构造仅出现在测试（`hooks.rs:1422,1452,1482`…）；`config.rs`/`app_settings.rs` 无任何 hook 键；前端仅透传 `hookTraceRecords`（`src/stores/runtime.ts:2560-2582`），无钩子配置 UI | `NoopHook\|HookExecutor\|PreToolUse\|PostToolUse\|hook_command\|CommandHook\|ShellHook\|hooks\.json\|HookSpec`；`Command::new\|std::process`（限 hooks.rs）；`AgentHookDescriptor {`；`hook`（config.rs/app_settings.rs/*.ts） |
| C05 | **CONFIRMED** | 文件仅 213 行，纯文本启发式：`agent/lsp.rs:132` `lsp_definition` 用 `format!("fn {} ", ident)` / `"struct {} "` / `"let {} "` 逐行 `line.contains(...)` 匹配（`:140-152`），references 用 `line.find(&ident)`（`:169-178`），hover 直接吐 `` format!("Symbol: `{}` (analyzed via local LSP AST/heuristic)") ``（`:190`）；diagnostics 恒空 `lsp.rs:205-212`（`diagnostics: Vec::new()`）。四个入口全部硬编码 `fallback_text_search: true`（`lsp.rs:160,185,201,211`）。无真实 LSP 客户端：`Cargo.toml`/`crates/pony-agent-core/Cargo.toml` 对 `lsp` **零依赖**（无 lsp-types/tower-lsp/jsonrpc-stdio），无进程派生、无 `Content-Length` 头 | `lsp_definition\|lsp_references\|lsp_hover\|lsp_diagnostics\|fallback_text_search`；`lsp`（三个 Cargo.toml）；`lsp_definition\|lsp_hover\|lsp_diagnostics`（tools.rs 注册面） |
| C06 | **CONFIRMED** | 超预算即硬失败，无落盘：`agent/dispatcher.rs:1099` 与 `:1434` `DispatchError::new("output_budget_exceeded", …)`；`budget.rs:129-146` 只有计数/并发/字节/时限预算，无「溢出写文件」路径。大文件读取直接拒绝并让模型改用分段读：`agent/tools.rs:2700-2714`（`file_too_large` + 「请改用 workspace_read_file_segment」），阈值 `tools.rs:158` `MAX_FULL_READ_BYTES = 120_000`。命令输出在环形缓冲被截断丢弃并只计数 dropped bytes（`agent/process.rs:1435-1472` 断言 `stdout_truncated` / `drain_stats.dropped`），无 artifact 文件。`spill\|offload\|output_path\|save_output\|content_ref\|body_ref` 全库零命中 | `spill\|offload\|truncat.*file\|large_output\|output_limit\|max_output_bytes\|persist.*output`；`artifacts?/\|content_ref\|body_ref\|回读\|excerpt\|head_tail\|clip.*output`；`output_budget_exceeded`；`write.*output.*file\|output_path\|save_output` |
| C07 | **CONFIRMED** | 文件级回滚标记恒为假且从不置真：`agent/hooks.rs:240` 字段 + `hooks.rs:1773` `workspace_rollback_capable: false`、`agent/session/store.rs:3826,3843`；全库无 `workspace_rollback_capable: true` / `applied: true`。所谓 checkpoint 只有两类非文件语义：执行/图中断点（`src-tauri/src/lib.rs:435-440` `load_execution_checkpoint`、`control_plane/graph_projection.rs`）与对话分叉（`control_plane/history_commands.rs:225-256` `fork_from_history_node` → `sessions.fork_from_history_node` 只快照消息状态）。无 `git stash`/`worktree`/shadow copy/`.bak` 备份用于工作区（`backup` 命中仅 `config.rs:555-580` 用于「provider registry 文件损坏备份」，与工作区回滚无关） | `checkpoint\|rewind\|snapshot\|restore.*file\|rollback`；`backup\|\.bak\|shadow_copy\|git stash\|worktree\|file_snapshot\|restore_file`；`workspace_rollback_capable`；`rewind\|rollback\|checkpoint`（*.vue/*.ts/src-tauri） |
| C08 | **CONFIRMED** | 无任何自动化驱动：`playwright\|puppeteer\|cdp\|chrome.?devtools\|webdriver\|headless\|chromium` 在 `crates/pony-agent-core/src` + `src-tauri/src` **零命中**（唯一 `headless` 词根出现在被排除的旁挂调研文档 `obscura-headless-browser-research.md`，是"调研报告"而非实现）；`src/**/*.ts|*.vue` 对 `playwright\|puppeteer\|cdp\|webdriver` 零命中。`screenshot` 命中全是提示词/上下文规则（`agent/context.rs:1034,1047,1257`），反而在禁止模型假装看图。`src-tauri/src/platform.rs:132-141,279` 是「打开系统浏览器 + host 白名单」，非浏览器控制。`src/lib/runtime/browser-preview.ts:1-2` 自述为「浏览器预览（无 Tauri 宿主）时的默认工具/capability 兜底」，纯 UI 假数据 | `playwright\|puppeteer\|cdp\|chrome.?devtools\|webdriver\|screenshot\|headless\|chromium`；`\bbrowser\b\|webview.*navigate\|selenium\|automation`；`playwright\|puppeteer\|cdp\|webdriver`（src *.ts/*.vue） |
| C09 | **CONFIRMED（附限定）** | 无用户可配 allow/deny 名单：`disabled_tools\|enabled_tools\|tool_allow\|tool_allowlist\|allow_rules\|deny_rules\|permission_rule` 全库零命中；配置页工具 tab 明写「当前运行时可用的工具目录（**只读**）」（`src/components/config/ConfigToolsSection.vue:45`），仅展示 `permissionFacts`。**限定（原结论"仅路径权限"不精确）**：存在工具级权限*模型*——`agent/dispatcher.rs:113-145` `ToolPolicyEvaluator` + `PermissionVerdict{WaitingHost,ApprovalRequired,Allow}` + descriptor 的 `requires_approval`/`host_mediated`/`scopes`；但生产 evaluator 全放行：`agent/governed_executor.rs:66-95`（`LegacyCompatiblePolicyEvaluator`，除 Ask 外 `PermissionVerdict::Allow`，注释自认 "real approval semantics replace this at the permission-contract milestone"），注册点 `governed_executor.rs:178`。故实际生效的工具级 allow/deny 规则＝不存在 | `allow_rules\|deny_rules\|permission_rule\|tool_allowlist\|allowed_tools\|deny_list\|allowlist\|PermissionMode\|tool_permission`；`disabled_tools\|enabled_tools\|tool_allow\|allowed_skill\|disabledTools\|enabledTools`；`ToolPolicyEvaluator\|PermissionVerdict` |
| C10 | **CONFIRMED** | 无 plan 模式门禁：`plan_mode\|PlanMode\|plan_gate\|read_only.*mode` 在 Rust 侧零命中；前端 `planMode\|isPlan\|计划模式` 零命中。`Plan` 是会话级任务清单状态机、明确「never executes arbitrary child calls」，只有 `Draft/Executing/Completed/Aborted` + step 状态 CAS（`agent/plan_state.rs:1-23`；`control_plane/ask_plan_commands.rs:101-183` 的 `plan_create/replace/merge/complete_step/list`），不含「禁写」语义。最接近的两处均非模式门禁：`agent/tool_runtime.rs:152-176` `TurnToolView` 是「本轮可见工具集过滤」，`tools.rs:161-168 READ_ONLY_BATCH_CHILD_PRIMITIVES` 是「batch 只允许只读子调用」的白名单（供 workspace_batch 用），二者都不因「进入计划模式」而禁止写工具 | `plan_mode\|PlanMode\|read_only.*mode\|plan.*readonly\|readonly.*plan\|plan_gate`；`计划模式\|只读模式\|plan.*forbid\|forbid.*write`；`planMode\|plan_mode\|isPlan\|计划模式`（src *.ts/*.vue）；`TurnToolView` |
| C11 | **CONFIRMED** | 无 USD 计价与硬上限：`usd\|maxBudget\|budget_usd\|cost_usd\|price.*token\|pricing\|per_1k\|token_price\|spend` 在 Rust 源码零实质命中（唯一 `cost` 是 CLI 斜杠命令）。`/cost` 是空壳：帮助文案承诺 "Show total tokens used and **cost estimates**"（`bin/cli.rs:411`），实现在 `bin/cli.rs:516-521` 只打印 Session ID / Model / `Rate: Standard polyllm metered tariff` —— **不算钱、不算 token、不设限**。`agent/budget.rs:40-72` `DispatchBudgetConfig` 的预算是调用数/并发数/嵌套深度/输出字节/控制请求 TTL，**无金额维度**；`agent/recovery.rs` 的"熔断"是死循环/连续失败熔断，非成本熔断。前端 `budget` 命中全是 `reasoningBudgetTokens`（思维链预算，`src/types/provider.ts:49`），与计费无关 | `usd\|maxBudget\|budget_usd\|cost_usd\|price.*token\|pricing\|金额\|计费\|熔断`；`\bcost\b\|pricing\|per_1k\|token_price\|spend`；`\bcost\b\|usd\|budget`（src *.ts/*.vue） |
| C12 | **CONFIRMED** | 无 OTel/OTLP：`opentelemetry\|otlp\|otel\|tracing_subscriber\|span.*export` 在 `crates/pony-agent-core/src`、`src-tauri/src` **零命中**；三个 `Cargo.toml`（根 / core / src-tauri）对 `opentelemetry\|otlp\|tracing-subscriber` 零依赖（`grep` exit=1），也无 `tracing` crate。存在的是自研进程内遥测：`agent/telemetry.rs`（trace/batch 观测记录与 `batch_child_plan`），以及 `agent/trace_persistence.rs` 落本地会话记录——不是 OTLP 导出协议 | `opentelemetry\|otlp\|otel\|tracing_subscriber\|span.*export`；`opentelemetry\|otlp\|tracing-subscriber`（Cargo.toml ×3）；`telemetry\|trace_persistence` |
| C13 | **CONFIRMED** | 无 cron/定时：`cron\|scheduler\|scheduled_task\|timer.*repeat\|interval.*job\|定时任务\|periodic` 在 Rust 侧零实质命中（`scheduler` 仅出现在 `hooks.rs:2159` 测试名与 `sandbox.rs:18` 注释）；`agent/jobs.rs` 是进程作业回收（`:238` reap 等待），`agent/event_bus.rs` 是同步事件分发，均无「按时刻/周期触发」。前端 `periodic` 仅 `src/stores/ask.ts:387` 的 Ask 轮询自终止，非调度器。无持久化任务表/schedule 配置项 | `cron\|scheduler\|scheduled_task\|timer.*repeat\|interval.*job\|心跳调度`；`schedul\|defer\|定时\|delay\|set_timeout`（jobs.rs/event_bus.rs）；`reminder\|定时任务\|schedule_task\|cron_\|every_day\|periodic` |
| C14 | **CONFIRMED** | 对外暴露只有 Tauri IPC：`src-tauri/src/lib.rs:1134` `tauri::Builder::default()` + `:1172` `.invoke_handler(tauri::generate_handler![…])`（86 个 `#[tauri::command]`），进程内无监听端口。`webhook\|acp\b\|agent.?client.?protocol\|grpc\|jsonrpc.*server\|axum\|actix\|hyper::Server\|openapi` 在 `crates/pony-agent-core/src`、`src-tauri/src` 零命中（`CP_ACP` 是 Windows 代码页常量，误命中）；`src`/`src-tauri` 的 `webhook\|acp\|mcp.?server\|openapi` 零命中。`TcpListener\|bind(\|listen(` 仅出现在测试 mock server（`agent/runtime/tests.rs:29,658`）。`src-tauri/src/turn_task_registry.rs:1-40` 是「同进程并行对话上限」注册表（内部并发治理），不是 HTTP/协议服务。三个 `bin/`（cli、tui、non_tauri_harness）是命令行前端，非 SDK | `webhook\|acp\b\|agent.?client.?protocol\|grpc\|jsonrpc.*server\|axum\|actix\|hyper::Server\|openapi`；`TcpListener\|bind(\|listen(\|HttpServer\|localhost.*port\|serve(`；`invoke_handler\|generate_handler`；`webhook\|\bacp\b\|openapi`（src、src-tauri） |
| C15 | **CONFIRMED** | 名为 PTY 实为管道：`agent/terminal.rs:1` 注释自称 "Interactive persistent terminal (**PTY**) tool family (PA-104)"，但派生用 `terminal.rs:407` `Command::new` + `:412-414` `stdin/stdout/stderr(Stdio::piped())`，无伪终端设备。全库 `openpty\|forkpty\|posix_openpt` **零命中**；`conpty\|CreatePseudoConsole\|PseudoConsole` 零命中；依赖侧无 `portable-pty`/`nix`（core `Cargo.toml` 仅 `libc`，windows-sys features 无 `Win32_System_Console`）。会话隔离靠进程组 `libc::killpg`（`terminal.rs:210-221,261`）。故交互式 CLI/TUI、密码提示、ANSI 光标类程序行为不等价 PTY | `\bpty\b\|portable_pty\|portable-pty\|conpty\|openpty\|pseudo.?terminal\|伪终端`；`openpty\|forkpty\|posix_openpt`（全库）；`Stdio::piped\|Command::new`（terminal.rs）；`pty\|nix`（Cargo.toml） |

统计：CONFIRMED 13 / REFUTED 1（C03，且仅部分子句）/ UNVERIFIED 0。

## 可回放命令（原文）

```bash
cd /home/dm/pony-agent

# C01
grep -rn "多个工具调用" crates/pony-agent-core/src/agent/provider/mod.rs
grep -rn "into_iter()" crates/pony-agent-core/src/agent/provider/openai_sse.rs | head -3
grep -rn "pub tool_call: Option<ToolCall>" crates/pony-agent-core/src/agent/provider/mod.rs
grep -rniE "join_all|join_set|spawn_blocking|rayon|tokio::spawn|concurren" --include=*.rs crates/pony-agent-core/src | head -40
grep -rniE "stream.*tool_exec|early.*execut|execute.*while.*stream|preempt.*tool" --include=*.rs crates/pony-agent-core/src | head -10

# C02
sed -n 195,240p crates/pony-agent-core/src/agent/orchestration.rs
grep -rniE "spawn_subagent|derive_agent|child_session|session_fork|sub_session|delegate_agent|agent_spawn" --include=*.rs crates/pony-agent-core/src src-tauri/src
grep -rn "spawn_teammate\|TeamTaskItem\|teammate" --include=*.rs crates/pony-agent-core/src | grep -v orchestration.rs

# C03
grep -rn "FakeMcpTransport\|StdioMcpTransport\|pub trait McpTransport" crates/pony-agent-core/src/agent/tool_runtime.rs
sed -n 393,462p crates/pony-agent-core/src/agent/tool_runtime.rs
sed -n 1,40p crates/pony-agent-core/tests/hotloader_and_mcp_contract.rs
grep -rn "McpResourceSurface" --include=*.rs crates/pony-agent-core/src src-tauri/src | head -20
grep -rniE '"initialize"|tools/list|list_tools' --include=*.rs crates/pony-agent-core/src src-tauri/src

# C04
grep -n "pub struct NoopHookExecutor\|pub trait AgentHookExecutor" crates/pony-agent-core/src/agent/hooks.rs
grep -rn "NoopHookExecutor" crates/pony-agent-core/src/agent/runtime/mod.rs
grep -rn "Command::new\|std::process" crates/pony-agent-core/src/agent/hooks.rs
grep -rn "AgentHookDescriptor {" --include=*.rs crates/pony-agent-core/src

# C05
sed -n 130,213p crates/pony-agent-core/src/agent/lsp.rs
grep -n "lsp" Cargo.toml crates/pony-agent-core/Cargo.toml      # exit=1，零依赖

# C06
grep -rn "output_budget_exceeded" --include=*.rs crates/pony-agent-core/src/agent/dispatcher.rs
sed -n 2698,2725p crates/pony-agent-core/src/agent/tools.rs
grep -rniE "spill|offload|output_path|save_output|content_ref|body_ref" --include=*.rs crates/pony-agent-core/src | head -20

# C07
grep -rn "workspace_rollback_capable" -r crates/pony-agent-core/src src-tauri/src
sed -n 220,260p crates/pony-agent-core/src/agent/control_plane/history_commands.rs
grep -rniE "backup|\.bak|shadow_copy|git stash|worktree|restore_file" --include=*.rs crates/pony-agent-core/src | grep -viE "test" | head -20

# C08
grep -rniE "playwright|puppeteer|cdp|chrome.?devtools|webdriver|headless|chromium" --include=*.rs crates/pony-agent-core/src src-tauri/src
grep -rniE "playwright|puppeteer|cdp|webdriver" --include=*.ts --include=*.vue src
sed -n 1,3p src/lib/runtime/browser-preview.ts

# C09
grep -rniE "disabled_tools|enabled_tools|tool_allow|allow_rules|deny_rules|permission_rule" --include=*.rs --include=*.ts crates/pony-agent-core/src src
sed -n 60,100p crates/pony-agent-core/src/agent/governed_executor.rs
grep -n "register_policy_evaluator" crates/pony-agent-core/src/agent/governed_executor.rs
sed -n 40,50p src/components/config/ConfigToolsSection.vue

# C10
grep -rniE "plan_mode|PlanMode|plan_gate|read_only.*mode" --include=*.rs crates/pony-agent-core/src
grep -rniE "planMode|plan_mode|isPlan|计划模式" --include=*.ts --include=*.vue src
sed -n 1,23p crates/pony-agent-core/src/agent/plan_state.rs

# C11
grep -n "cost estimates" crates/pony-agent-core/src/bin/cli.rs
sed -n 516,522p crates/pony-agent-core/src/bin/cli.rs
grep -rniE "usd|maxBudget|cost_usd|price.*token|pricing" --include=*.rs crates/pony-agent-core/src | head -20
sed -n 40,72p crates/pony-agent-core/src/agent/budget.rs

# C12
grep -rniE "opentelemetry|otlp|otel|tracing_subscriber" --include=*.rs --include=*.toml crates/pony-agent-core src-tauri
grep -rinE "opentelemetry|otlp|tracing-subscriber" Cargo.toml crates/pony-agent-core/Cargo.toml src-tauri/Cargo.toml   # exit=1

# C13
grep -rniE "cron|scheduler|scheduled_task|timer.*repeat|interval.*job" --include=*.rs crates/pony-agent-core/src src-tauri/src
grep -rniE "reminder|定时任务|schedule_task|cron_|every_day|periodic" --include=*.rs --include=*.ts crates/pony-agent-core/src src src-tauri/src

# C14
grep -n "invoke_handler\|Builder::default" src-tauri/src/lib.rs
grep -rniE "webhook|acp\b|agent.?client.?protocol|grpc|axum|actix|hyper::Server|openapi" --include=*.rs crates/pony-agent-core/src src-tauri/src
grep -rniE "TcpListener|bind\(|listen\(|HttpServer|serve\(" --include=*.rs src-tauri/src crates/pony-agent-core/src | head -15

# C15
sed -n 400,420p crates/pony-agent-core/src/agent/terminal.rs
grep -rn "openpty\|forkpty\|posix_openpt" --include=*.rs crates src-tauri   # 零命中
grep -rn "PseudoConsole\|CreatePseudoConsole" --include=*.rs crates/pony-agent-core/src src-tauri/src   # 零命中
sed -n 30,50p crates/pony-agent-core/Cargo.toml
```

## 追加发现

**G-01（纠正性）治理型工具执行器已落地并在生产路径接线，不能算「无执行器」。** `agent/runtime/builder.rs:78-84` 默认工具引擎即 `build_governed_executor(...)`；该 dispatcher 提供结构化 preflight、原子预算账本（调用数/并发/字节/时限/取消，`budget.rs:80-221`）、最大嵌套深度、cycle 拒绝、`std::thread::scope` 并发子派发（`child_dispatch.rs:249`）、fail-closed 沙箱（`governed_executor.rs:14-21` 注释 + `agent/sandbox.rs`）。C01 若表述为「无并发执行器」会被这条反证；准确表述应是「provider 级多工具调用不受支持（单调用模型 + Anthropic 报错 + OpenAI 静默丢首）」。

**G-02（应单列缺陷，非「欠缺」）内存桩被暴露成 model_visible 工具。** `subagent`/`workflow`/`spawn_teammate`/`list_agents`/`send_message`/`interrupt_agent`/`drain_inbox`/`team_task_*` 全部注册进模型可见工具面（`provider/mod.rs:4645-4700`、`tools.rs:1188,1262,5453-5548`），但实现是「合成字符串 + 进程内静态 HashMap」（`orchestration.rs:200,223,236,279`）。模型会真诚地相信自己派生了子代理。这比"能力缺失"更危险，建议按「假能力暴露」定性并优先处理。

**G-03 MCP 的真实缺口比原结论窄也更具体。** 真实：`tool_runtime.rs:395` stdio 传输能跑通 JSON-RPC 往返（集成测试 `tests/hotloader_and_mcp_contract.rs:25`）。缺口是：(a) 无 `initialize` / `tools/list` 握手，method 名自造 `list_resources`/`list_resource_templates`/`read_resource`（`tool_runtime.rs:452-458`），非 MCP spec 的 `resources/list`/`resources/read`；(b) 每次请求 spawn 一个进程、单行读写，无长连接会话/能力协商；(c) 无 SSE/HTTP/streamable 传输；(d) `McpResourceSurface` 未接线，`mcp_resource_read` 当前 fail-closed 返回 source_unavailable（`runtime/tool_exec.rs:327-333`）。即「有传输、无客户端」。

**G-04 `terminal_*` 是真实且相当完备的持久交互式终端工具族，但确实不是 PTY。** `agent/terminal.rs`（646 行）提供 `terminal_open/send/read/signal/close`，2MB 环形缓冲、最多 16 会话、Windows Job Object / POSIX 进程组收容、cwd 越界 fail-closed，并有黑盒验收 `tests/pa104_terminal_acceptance.rs`。若审计把「无终端能力」列为缺口即误判；正确缺口表述是「无伪终端（ConPTY/openpty），交互式全屏程序/密码提示不可靠」。

**G-05 工具权限：有模型、无规则、生产全放行。** `dispatcher.rs:113-145` 的 `ToolPolicyEvaluator` + `PermissionVerdict{Allow,ApprovalRequired,WaitingHost}` + descriptor 的 scopes 是真实机制，且 Ask 工具确实走 host 中介；但默认注册的 `LegacyCompatiblePolicyEvaluator`（`governed_executor.rs:66-95,178`）除 Ask 外一律 `Allow`，代码注释自认审批语义"待 permission-contract 里程碑"。因此 C09 结论成立应理解为「无可配置工具级 allow/deny，且门禁在生产路径被兼容层绕过」，而不是「完全没有权限模型」。

**G-06 原结论未列、但实际存在的能力（防止反向误判为缺口）**：`web_search_query`/`web_fetch_url`（`agent/web_access.rs`，含 host/端口 deny list `:749`）、文档转换 anydoc（`document_conversion.rs`，cap 512KB `:26`）、图片引用式读取 `view_image`（`image_artifact.rs`）、patch 应用（`patch.rs`）、git 检索（`git_search.rs`）、崩溃上报 PonySentry（`ponysentry/`，`src-tauri/src/main.rs:4-5` 装 panic hook）、SQLite 会话持久化（`sqlite_session.rs`）、对话级 history fork（`control_plane/history_commands.rs:225`）、`workspace_batch` 并行只读子调用（≤24）。

**G-07 KPI 级真实缺口（本次检索顺带发现，值得单独立项）**：(1) 无文件级快照/回滚，写操作不可撤销（C07 的直接后果）；(2) 无输出落盘溢出，大输出只能截断或让模型分段重读（`tools.rs:2700-2714`），长日志场景会反复重跑命令；(3) 无成本上限，`/cost` 是空壳且帮助文案过度承诺（`cli.rs:411` vs `:516-521`）；(4) 无 plan 只读门禁，模型一旦进入"规划"话术仍可直接改文件。
