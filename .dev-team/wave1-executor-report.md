# Wave 1 T2：product-bug 最小产品修复报告（Executor）

> 产出：wave1-executor（T2）；基线 HEAD：20d31be（T1 测试修正冻结）+ 后续 ledger/文档提交
> 依据：.dev-team/wave1-triaged-tests.md 分诊矩阵 product-bug 条目（#3-9、#10、#12、#13-14）
> 约束遵守：零测试代码改动；git 零写操作；写域扩展（Lead 批准 provider/openai_sse.rs 一处函数）；未触碰 process.rs / Vue 删除（T3 处置已由 Lead 提交）

## 一、修复清单（4 项，3 文件，129+/21-）

### ① tools.rs `workspace_command_parts_with_batch`（Windows 批处理 spawn 路径剥 `\\?\` 前缀）
- **对象**：#6、#7、#8、#9（run_command C 类）、#13、#14（E 类）共同根因 [WINDOWS-CI-ONLY]
- **改动**：Windows 分支 `vec!["/C", batch_path.display()]` → `vec!["/C", windows_shell_path(&batch_path.display())]`。批处理内容中 cwd 早已剥前缀，但 spawn 行 `/C <批处理文件自身路径>` 未剥——canonicalize 产物的 `\\?\C:\...` 前缀 cmd.exe 无法执行（无空格 ws：The system cannot find the path specified；含空格 ws：cmd 引号算法在空格处截断 `'\\?\C:\...\pony' is not recognized`）。
- **静态推演（Windows）**：剥前缀后 `C:\...\pony-agent-run-{session}.cmd` 可被 cmd /C 执行；含空格路径由 std 引号包裹、cmd /C 引号算法规则 1 按可执行文件路径保留引号；`exit /b 7` 立即退出批处理使 errorlevel=7 进入 cmd 退出码（#7）；`ping -n 6` 启动后 200ms kill 计时器触发 → timeout（#9）；临时文件删除仍走原 `\\?\` 路径（fs::remove_file 支持扩展路径，#13 清理断言不受影响）。

### ② tools.rs `canonicalize_workspace_target`（canonicalize 失败分支词法逃逸判定）
- **对象**：#3、#4、#5（路径权限 B 类）[WINDOWS-CI-ONLY]
- **改动**：`candidate.canonicalize()` 失败时不再直接返回 invalid_path，先做 `lexically_within_root(&root, &candidate)`（新增辅助函数：剥 `\\?\` 前缀 → 组件级折叠 `..`/`.`（卷根之上钳制）→ 组件前缀比较，Windows 大小写折叠）：词法上已逃逸 root → 返回 `requires_authorization`（PA-080 宿主审批语义）；仍在 root 内（如工作区内不存在的文件）→ 维持 invalid_path。
- **静态推演（Windows）**：`../../../etc` 越过卷根/目标不存在 → canonicalize 失败 → 词法逃逸 → requires_authorization；`does-not-exist.txt` 在 root 内 → invalid_path 不变。

### ③ provider/mod.rs `extract_openai_tool_call`（空洞守卫收窄，sync 路径）
- **对象**：#10 run_turn_repairs_blank_tool_name_before_execution（Linux 可复现红，已转绿）
- **改动**：46040ae 的"空名一律返回 None"守卫收窄为**仅"空名 + 空参数"空洞调用**返回 None；空名但参数非空时保留 `name=""` 的 ToolCall，交 normalize_tool_directive 的 `infer_tool_name_from_arguments` 修复路径。新增 `is_hollow_tool_arguments` 辅助（语义同 runtime/stream_support.rs `is_empty_tool_arguments`）。对照 46040ae 提交信息："empty name+empty args is dropped；non-empty args keep the existing repair/error path"——本次只收窄到该语义。

### ④(a) provider/openai_sse.rs `partial_openai_tool_call_to_tool_call`（SSE 流守卫收窄，写域经 Lead 批准扩展）
- **对象**：#12 start_turn_stream_repairs_blank_tool_name_in_followup_stream（Linux 可复现红，已转绿）
- **改动**：与 ③ 同构收窄。修复前：followup-stream 中"仅 reasoning + 空名非空参数 tool call、无 content"→ 空名被丢弃 → `finish()` 报"未提取到文本内容"→ 回退 sync → sync 把 mock 的 SSE 响应当纯 JSON 解析失败 → local fallback 文本 ≠ 期望。修复后：空名非空参数 tool call 保留 → finish() 有 tool_call 不再失败 → runtime 修复为 Read → 执行 → 第三请求返回最终文本。

## 二、本地机器证据（cargo test -p pony-agent-core --target-dir target-test，Linux）

| 用例 | 结果 |
|---|---|
| agent::runtime::tests::run_turn_repairs_blank_tool_name_before_execution（#10，修复前红） | ok，0.16s |
| agent::runtime::tests::start_turn_stream_repairs_blank_tool_name_in_followup_stream（#12，修复前红） | ok，0.16s |
| agent::tools::tests::run_command 系列（含 #6-9 四用例 + fails_closed 2 + deny 2 + cwd 1） | 9 passed |
| agent::tools::tests::list_files_rejects_path_traversal / path_info_rejects_invalid_path / read_file_rejects_path_traversal（#3-5） | 3 passed |
| agent::tools::tests 全模块 | 118 passed，0 failed |
| agent::provider::tests 全模块 | 51 passed，0 failed |
| agent::runtime::tests::run_turn 前缀 | 17 passed |
| agent::runtime::tests::start_turn_stream 前缀（--test-threads=1） | 20 passed（含 completes 与 #12） |
| agent::runtime::tests::empty_tool_call_*（46040ae 空洞回归） | 2 passed |
| agent::sandbox / agent::governed_executor | 6 / 11 passed |
| RUSTFLAGS="-D warnings" cargo check --tests | exit 0，无警告 |

注：start_turn_stream 前缀默认并行跑曾触发挂起——为 T1 已定位的既有顺序依赖问题（MockHttpServer 对超量请求无 accept 兜底，见分诊矩阵第四节），与本次改动无关；该前缀单线程 20/20 通过，completes 单测单跑 0.19s 通过。

## 三、需 CI 复跑验证清单（[WINDOWS-CI-ONLY]，本地 Linux 无法执行）

| 测试 | 修复项 | 静态推演结论 |
|---|---|---|
| #6 run_command_executes_when_a_registered_sandbox_backend_is_available | ① | 批处理可执行 → echo hello exit 0 → ok |
| #7 run_command_returns_error_for_non_zero_exit_code | ① | exit /b 7 立即退出 → exitCode 7 → non_zero_exit |
| #8 run_command_returns_stdout_and_exit_code | ① | 同 #6，stdout 含 hello |
| #9 run_command_returns_timeout_error_when_command_exceeds_deadline | ① | ping 启动后 200ms 杀 → code=timeout |
| #13 windows_run_powershell_command_with_quotes_returns_real_stdout_and_cleans_batch | ① | 引号原样写入批处理 → 真实 stdout、exit 0、.cmd 被删 |
| #14 windows_run_powershell_in_workspace_root_with_spaces_returns_real_stdout | ① | 剥前缀 + std 引号包裹 → 含空格路径可执行 |
| #3 list_files_rejects_path_traversal | ② | canonicalize 失败 → 词法逃逸 → requires_authorization |
| #4 path_info_rejects_invalid_path | ② | 同上 |
| #5 read_file_rejects_path_traversal | ② | 同上 |

## 四、④(b) 静态结论（按 Lead 指示不做，留说明）

- **为什么 #12 不再触发 sync fallback**：修复 (a) 后 followup-stream 直接提取到（空名待修复的）tool call，`finish()` 不再报"未提取到文本内容"，无需回退 sync；机器证据：本地 #12 转绿、请求数为 3（初始 + 修复后工具 followup + 最终 followup）。
- **sync fallback 仍可能收到 SSE 包装响应的条件**：某网关对 followup-sync（body `stream:false`）仍返回 SSE 帧（`data: {...}\n\ndata: [DONE]`）时，`post_openai_json` 的纯 JSON 解析失败 → 走 local fallback 文本（graceful 降级，不崩溃，但丢失真实模型回复）。
- **建议后续**（不在本轮最小闭包内）：`post_openai_json` 解析失败时若正文以 `data:` 开头，用 openai_sse 累积器重建 JSON 载荷（`{"choices":[{"message":{...}}]}`）再走既有提取；或单独在 followup-sync 入口加 SSE 兜底。涉及共享同步路径，需独立评估。

## 五、工作区状态

仅 3 个产品文件改动（未提交，git 写由 Lead 执行）：
- crates/pony-agent-core/src/agent/tools.rs（① + ②，+104/-6）
- crates/pony-agent-core/src/agent/provider/mod.rs（③，+28/-4）
- crates/pony-agent-core/src/agent/provider/openai_sse.rs（④a，+18/-7）

---

# Wave 1 T4（⑤）：Run 启动后关闭 stdin（Windows PS 无控制台挂起修复）

> 依据：Lead CI 复跑结论——14 失败降至 2（1001 passed），剩余 #13/#14 从"批处理无法执行"变为 30s 超时挂起；诊断：PS 5.1 在 CREATE_NO_WINDOW + 未关闭 stdin 管道下 ConsoleHost 阻塞等待输入。写域：tools.rs + process.rs（Lead 账本批准）。

## 改动（2 文件，22+）

1. **process.rs**：ProcessManager 新增固有方法
   `pub fn close_stdin(&self, session_id: &str, handle: &str) -> Result<(), String>`：
   `lookup` 后 `take()` 掉 `entry.stdin`（drop ChildStdin → 子进程 stdin 收 EOF）；已无管道时 Err
   （幂等容忍由调用方决定）；锁与错误风格参照 `write_stdin_inner`（process.rs:804 一带）。
2. **tools.rs run_command**：`process_manager.start` 成功后、`kill_after` 之前立即
   `let _ = self.process_manager.close_stdin(&session_id, &handle);`（错误忽略），中文注释说明动机：
   Run 从不写 stdin；无控制台控制台程序等待输入会挂起，EOF 打破阻塞。

## 本地机器证据（Linux）

- `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --target-dir target-check --tests`：exit 0
- `cargo test -p pony-agent-core --target-dir target-test -- run_command 系列`：9 passed
- `agent::process::tests`：14 passed（write_stdin 用例不受影响）
- `agent::tools::tests` 全模块回归：118 passed

## 静态推演（Windows）

stdin EOF 后 PS 5.1 ConsoleHost 不再等待交互输入 → `-Command` 正常执行退出 → #13/#14 转绿；
顺带覆盖任何读 stdin 的用户命令的潜在挂起（Run 本就该给 EOF）。需 CI 复跑验证。
