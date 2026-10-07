# Wave 1 T1：14 个 CI 失败 core 测试分诊矩阵 + 测试侧修正

> 产出：wave1-tester（Test Agent）
> 基线 commit：805541e (v0.1.108, main)；CI 失败清单与 v0.1.107 完全一致（14 个，见 .dev-team/wave1-evidence.md）
> 本地机器：Linux（无法运行 #[cfg(windows)] 测试，相关结论标注 [WINDOWS-CI-ONLY]）
> 约束遵守：仅修改 #[cfg(test)] 测试代码；产品代码零改动；无 git 写操作。

## 一、14 条分诊矩阵

| # | 测试（模块:行） | 类别 | 分诊 | 依据（git 历史 / 实现意图） | 处置 |
|---|---|---|---|---|---|
| 1 | anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools (provider/mod.rs:4623) | A | **stale-expectation** | CI left 实际工具面已含 9 个 team 工具（2ce5d1c feat(orchestration): wire subagent and agent team tools 加入；provider 日志 tools=31 为设计值），测试期望止于 ViewImage 过期 | 测试期望补齐 team 工具 → 已改，本地转绿 |
| 2 | openai_tools_payload_uses_product_tool_surface_for_builtin_tools (provider/mod.rs:4587) | A | **stale-expectation** | 同上（同一 builtin_tools() surface） | 测试期望补齐 team 工具 → 已改，本地转绿 |
| 3 | list_files_rejects_path_traversal (tools.rs:9937) | B | **product-bug [WINDOWS-CI-ONLY]** | 期望 requires_authorization 是 PA-080 意图（a1b75d8 + canonicalize_workspace_target 注释"workspace 外读 → classify_path(Read) → requires_authorization"）；CI 得 invalid_path。Linux 本地绿（../../../etc canonicalize 成功→逃逸→requires_authorization）；Windows 红：相对路径越过卷根时 canonicalize 直接失败→invalid_path，逃逸判定分支不可达 | 不改测试，交 Executor（Windows 路径判定顺序） |
| 4 | path_info_rejects_invalid_path (tools.rs:9870) | B | **product-bug [WINDOWS-CI-ONLY]** | 同上 | 不改测试，交 Executor |
| 5 | read_file_rejects_path_traversal (tools.rs:9709) | B | **product-bug [WINDOWS-CI-ONLY]** | 同上 | 不改测试，交 Executor |
| 6 | run_command_executes_when_a_registered_sandbox_backend_is_available (tools.rs:8973) | C | **product-bug [WINDOWS-CI-ONLY]** | 期望（注册可用沙箱→ok）是 phase-5 P1-2 门禁后的产品意图；Linux 绿、Windows 红（详见 #6-9 根因） | 不改测试，交 Executor |
| 7 | run_command_returns_error_for_non_zero_exit_code (tools.rs:8850) | C | **product-bug [WINDOWS-CI-ONLY]** | 期望 exit 7 得 1（Windows 批处理执行失败退 1） | 不改测试，交 Executor |
| 8 | run_command_returns_stdout_and_exit_code (tools.rs:8696) | C | **product-bug [WINDOWS-CI-ONLY]** | 期望 ok 得 error | 不改测试，交 Executor |
| 9 | run_command_returns_timeout_error_when_command_exceeds_deadline (tools.rs:8886) | C | **product-bug [WINDOWS-CI-ONLY]** | 期望 timeout 得 non_zero_exit（命令先失败退出） | 不改测试，交 Executor |
| 10 | run_turn_repairs_blank_tool_name_before_execution (runtime/tests.rs:4663) | D | **product-bug** | sync 路径 blank-name 修复断裂：extract_openai_tool_call 自 46040ae 起对 name="" 一律返回 None（即使 arguments 非空），normalize_tool_directive 修复路径不可达；46040ae 提交信息明确"non-empty args keep the existing repair/error path"（只应丢弃空名+空参数空洞调用）。Linux 本地复现红。测试期望符合 PA-095 意图 | 不改测试，交 Executor（修复点：extract 仅丢弃空名+空参数） |
| 11 | runtime_default_tool_executor_fails_closed_for_run_without_sandbox (runtime/tests.rs:3458) | D | **stale-expectation** | AgentRuntime::new() 自 b8cba07（register production sandbox backend）起生产装配总是注册 Native/HostApproved 后端（governed_executor.rs:187-221）；同库配套测试 governed_executor_registers_production_sandbox_backend_for_run 断言生产路径必须已注册后端、fail-closed 经 NoSandboxBackend override 入口验证。测试用 new() 构造"无沙箱"状态已不可达 → 期望过期 | 测试改为 builder + NoSandboxBackend override（保留 fail-closed 语义）→ 已改，本地转绿 |
| 12 | start_turn_stream_repairs_blank_tool_name_in_followup_stream (runtime/tests.rs:6538) | D | **product-bug** | 两层缺陷：(a) followup-stream 对"仅 reasoning+blank tool call、无 content"的 SSE 判定"未提取到文本内容"失败→回退 sync；(b) sync fallback 把 SSE 包装响应（data: {...}\n\ndata: [DONE]）当纯 JSON 解析失败（expected value at line 1 column 1）。Linux 本地复现红。测试期望符合意图 | 不改测试，交 Executor |
| 13 | windows_run_powershell_command_with_quotes_returns_real_stdout_and_cleans_batch (tools.rs:11206) | E | **product-bug [WINDOWS-CI-ONLY]** | 27da4cd/5515533 冻结的 Bug A 修复验收目标；CI exit 1、stderr "The system cannot find the path specified."。静态推演：临时批处理文件路径源自 canonicalize 产物 `\\?\C:\...`（run_command 只剥离了批处理**内容**的 cwd 前缀，未处理批处理文件**自身路径**的 `\\?\` 前缀），cmd /C 无法执行 → 全部 Windows Run 失败 | 不改测试，交 Executor，CI 复跑验证 |
| 14 | windows_run_powershell_in_workspace_root_with_spaces_returns_real_stdout (tools.rs:11276) | E | **product-bug [WINDOWS-CI-ONLY]** | 同上根因；含空格 workspace 下 stderr `'\\?\C:\...\pony' is not recognized`（cmd 引号算法在空格处截断 `\\?\` 前缀路径） | 不改测试，交 Executor，CI 复跑验证 |

**合计**：stale-expectation 3 条（#1、#2、#11，均已修正测试并本地转绿）；product-bug 11 条（其中 8 条 [WINDOWS-CI-ONLY]）。

### C/E 类共同根因（静态推演，[WINDOWS-CI-ONLY]）
run_command Windows 分支（tools.rs:1882-1904）写批处理文件 `<execution_root>/.tmp/pony-agent-run-{session}.cmd`，
而 execution_root 来自 `resolved_workspace_root`/`classify_workspace_path` 的 canonicalize 结果——Windows 上带 `\\?\` 前缀
（493da55 canonicalize/normalize 系列引入）。spawn `cmd /C <batch_path>` 时：
- 无空格 ws：cmd 尝试把 `\\?\C:\...cmd` 当可执行文件 → "The system cannot find the path specified."
- 含空格 ws：cmd 引号算法在空格处截断 → `'\\?\C:\...\pony' is not recognized`
证据：E1/E2 的 CI stderr 原样包含 `\\?\` 前缀路径；批处理**内容**中 cwd 经 windows_shell_path 剥离前缀（无此报错），
差异恰在批处理**路径本身**未剥离。C 类 4 个测试同为 Windows Run 全失败的直接后果。
（Executor 修复方向：spawn 前对 batch_path 剥 `\\?\` 前缀 + 确认引号形态，或改用非 canonicalize 前缀的受控 tmp 根。）

## 二、测试修正清单（仅 #[cfg(test)] 测试代码）

| 文件 | 测试 | 修正内容 | 本地验证 |
|---|---|---|---|
| crates/pony-agent-core/src/agent/provider/mod.rs | openai_tools_payload_uses_product_tool_surface_for_builtin_tools | 期望列表追加 9 个 team 工具（subagent/workflow/spawn_teammate/list_agents/send_message/interrupt_agent/drain_inbox/team_task_create/team_task_update/team_task_list），注释标注 2ce5d1c 来源 | cargo test：ok |
| crates/pony-agent-core/src/agent/provider/mod.rs | anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools | 同上 + 尾部断言 last_two 改为 team_task_update/team_task_list | cargo test：ok |
| crates/pony-agent-core/src/agent/runtime/tests.rs | runtime_default_tool_executor_fails_closed_for_run_without_sandbox | AgentRuntime::new() → AgentRuntimeBuilder::desktop().tool_executor(build_governed_executor_with_sandbox_backend(None,None,NoSandboxBackend)).build()，保留 fail-closed 语义（sandbox_unavailable） | cargo test：ok |

> 注意：修正内容均为工作区未提交状态；由 Lead 执行红相锚定提交。测试修正后，12 个非 Windows 失败中
> 3 个转绿，剩余 9 个（B×3 + C×4 + D×2）保持红相交 Executor；2 个 Windows 测试保持红相交 Executor + CI 验证。

## 三、本地机器证据（cargo test -p pony-agent-core --target-dir target-test）

修改后相关用例验证输出（节选）：
```
running 3 tests
test agent::provider::tests::anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools ... ok
test agent::provider::tests::openai_tools_payload_uses_product_tool_surface_for_builtin_tools ... ok
test agent::runtime::tests::runtime_default_tool_executor_fails_closed_for_run_without_sandbox ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 991 filtered out
```
product-bug 红相保留确认（修改后未动，仍红）：
- run_turn_repairs_blank_tool_name_before_execution：left "继续读取目标文件。" ≠ right "tauri.conf.json 已成功读取。"（Linux 复现）
- start_turn_stream_repairs_blank_tool_name_in_followup_stream：SSE data: {...} 被 sync fallback 当纯 JSON 解析失败（Linux 复现）
- 其余 B/C 类 Linux 全绿（Windows 特有失败，[WINDOWS-CI-ONLY]，需 CI 复跑确认）

## 四、附带调查：本地全量 --lib 测试挂起（只调查不修复）

详见下节（挂起定位结果）。
