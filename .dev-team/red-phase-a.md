# Stage 2 红相锚定：Windows 批处理执行契约（Bug A）

## 背景
- 根因：`crates/pony-agent-core/src/agent/tools.rs` 的 `workspace_command_parts()`（L6438）Windows 分支把
  含双引号的命令原样拼进 `cmd /C "cd /d <path> && <command>"` 单参数；Rust `std::process::Command` 按
  CommandLineToArgvW 把内嵌 `"` 转义成 `\"`，cmd.exe 不按该规则解析 → 引号失衡/管道误切、stdout 只回显
  命令文本、真实输出丢失。
- 修复方向（Executor 实现）：Windows 分支写入受控 tmp（`<root>/.tmp/`，参照 `controlled_tmp_dir()` L3648）
  下的唯一临时 .cmd 批处理文件（`@echo off\r\ncd /d "<cwd>"\r\n<命令原样>\r\nexit /b %errorlevel%\r\n`），
  以 `cmd /C <批处理文件路径>` 执行，结束后删除。
- 可测性：Executor 新增平台无关纯函数 `build_windows_batch_script(command: &str, cwd: &str) -> String`。

## 测试契约（crates/pony-agent-core/src/agent/tools.rs tests mod 末尾）
| 测试 | 依赖 | 期望 |
|---|---|---|
| A1 `windows_batch_script_emits_expected_lines_verbatim` | `build_windows_batch_script` | 首行 `@echo off`；含 `cd /d "C:\Users\test\work"`；命令含 `"Write-Output 'hello'"` 原样保留（无 `\"`）；含 `exit /b %errorlevel%` |
| A2 `windows_batch_script_preserves_cmd_metacharacters_verbatim` | `build_windows_batch_script` | `& | > "` 逐字保留；断言不含 `^&`/`^|`/`^>`、不含 `\"` |
| A3 `workspace_command_parts_non_windows_branch_is_unchanged` | `workspace_command_parts(两参)` | 非 Windows 返回 `("sh", ["-lc", "cd '<cwd>' && <command>"])`（若 Executor 改三参签名，按新签名调整并记录） |
| A4（`#[cfg(windows)]`，CI windows-latest 验证）`windows_run_powershell_command_with_quotes_returns_real_stdout_and_cleans_batch` | ToolRouter | exitCode==0 且 stdout 含 hello 且不含 "Write-Output"（无命令回显）；运行后 `.tmp/` 下无遗留 .cmd/.bat 临时文件 |

## 运行命令
```bash
cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test --package pony-agent-core --lib windows_batch_
```

## 输出片段（红相证据：A1/A2 因函数尚不存在 → 编译失败）
```
error[E0425]: cannot find function `build_windows_batch_script` in this scope
10884 |         let script = build_windows_batch_script(
error[E0425]: cannot find function `build_windows_batch_script` in this scope
10913 |         let script = build_windows_batch_script(command, r"C:\w");
error: could not compile `pony-agent-core` (lib test) due to 2 previous errors
（最终 exit code 101）
```

## 阻塞/注意事项
- 完整 `cargo test`（含集成测试目标）被未跟踪文件 `crates/pony-agent-core/tests/ask_user_acceptance.rs`
  的编译错误阻断（E0599/E0716，HEAD 上即存在，非本次改动引入，未触碰）。红相验证使用 `--lib`。
- A1/A2/A4 为修复后契约：A1/A2 在 Executor 落地 `build_windows_batch_script` 后转绿，
  A4 待 CI windows-latest 验证（本机 Linux 不编译不执行）。
- A3 依赖 `workspace_command_parts` 签名：已与 Executor 同步（详见 team message），
  若 Executor 采用三参签名，A3 调用将改为 `workspace_command_parts(cmd, cwd, None)`，断言不变。