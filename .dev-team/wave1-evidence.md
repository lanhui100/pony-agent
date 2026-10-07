# Wave 1 证据基线（CI core 测试遗留失败）

- 基线 commit: 805541e (v0.1.108, main)
- 相关 CI run: 37583508659（最近一次失败，verify 步骤 "Core 单元与集成测试"）、37570255012（v0.1.107 同步骤失败）、37495781561（v0.1.106，仅 8 个失败）
- 一手日志: /tmp/ci_failed.log (= /tmp/ci_now.log), /tmp/ci_107.log, /tmp/ci_106.log；测试名清单 /tmp/ci_now.names, /tmp/ci_106.names
- 结论：当前 14 个失败与 v0.1.107 时（弹窗修复前）完全一致（清单 md5 相同）；其中 9 个在批处理修复时代（v0.1.106→v0.1.107 之间）引入，5 个更早（v0.1.106 时代就有）。全部早于本次弹窗修复，属遗留问题。
- CI 该步骤命令：npm run cargo:test:shared -- --package=pony-agent-core（windows-latest, RUSTFLAGS=-D warnings 环境）；最近一次 989 passed / 14 failed / 19.17s 完成（无超时挂起，纯断言失败）。

## 14 个失败测试（五类）

### A. provider tool-surface 期望过期（2）
- agent::provider::tests::anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools (provider/mod.rs:4623)
- agent::provider::tests::openai_tools_payload_uses_product_tool_surface_for_builtin_tools (provider/mod.rs:4587)
- 实际 left 已含 team 工具：subagent, workflow, spawn_teammate, list_agents, send_message, interrupt_agent, drain_inbox, team_task_create, team_task_update, team_task_list；期望 right 止于 ViewImage。

### B. 路径权限错误码漂移（3）
- agent::tools::tests::list_files_rejects_path_traversal (tools.rs:9937)
- agent::tools::tests::path_info_rejects_invalid_path (tools.rs:9870)
- agent::tools::tests::read_file_rejects_path_traversal (tools.rs:9709)
- 期望 Some("requires_authorization")，实际 Some("invalid_path")。

### C. run_command 系列（4）
- agent::tools::tests::run_command_executes_when_a_registered_sandbox_backend_is_available (tools.rs:8973)：期望 ok 得 error
- agent::tools::tests::run_command_returns_error_for_non_zero_exit_code (tools.rs:8850)：期望 exitCode Some(7) 得 Some(1)
- agent::tools::tests::run_command_returns_stdout_and_exit_code (tools.rs:8696)
- agent::tools::tests::run_command_returns_timeout_error_when_command_exceeds_deadline (tools.rs:8886)：期望 kind=timeout

### D. blank-tool-name 运行期测试（3）
- agent::runtime::tests::run_turn_repairs_blank_tool_name_before_execution (runtime/tests.rs:4663)：期望 "tauri.conf.json 已成功读取。" 得 "继续读取目标文件。"
- agent::runtime::tests::start_turn_stream_repairs_blank_tool_name_in_followup_stream (runtime/tests.rs:6538)：期望 followup 文本，实际 provider 回退失败摘要（SSE data: {...} 被当纯 JSON 解析）
- agent::runtime::tests::runtime_default_tool_executor_fails_closed_for_run_without_sandbox (runtime/tests.rs:3458)：期望 error 得 ok（疑沙箱 fail-closed 回归）

### E. Bug A 批处理集成测试（2, #[cfg(windows)]，本地 Linux 无法运行）
- agent::tools::tests::windows_run_powershell_command_with_quotes_returns_real_stdout_and_cleans_batch (tools.rs:11206)
- agent::tools::tests::windows_run_powershell_in_workspace_root_with_spaces_returns_real_stdout (tools.rs:11276)

## 分诊矩阵输出位置
.dev-team/wave1-triaged-tests.md（Test Agent 产出）
