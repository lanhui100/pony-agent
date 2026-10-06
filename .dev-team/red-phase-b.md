# Stage 1 红相锚定：tool_result_failure_kind error:null 误分类（Bug B）

## 背景
- 根因：`crates/pony-agent-core/src/agent/runtime/types.rs` 的 `tool_result_failure_kind()`（L17-55）。
  `parsed.get("error")` 对 JSON `"error": null` 返回 `Some(Null)`，被 `if let Some(error) = ...` 当作"有 error"，
  `error.get("kind")` → `None` → `unwrap_or_default()` → `""` → 落入 `Some(CapabilityFailureKind::InvocationFailed)`。
- 后果：桌面 Windows 会话中 Run 成功（status=ok、exitCode=0）也被 UI 标成"调用失败 (invocation_failed)"。

## 测试契约（crates/pony-agent-core/src/agent/runtime/tests.rs，模块 `agent::runtime::tests::parity`）
| 测试 | 行号 | 期望 | 实际（修复前） |
|---|---|---|---|
| B1 `bug_b_ok_run_with_error_null_is_none_not_invocation_failed` | 9916 | `None` | `Some(InvocationFailed)`（红） |
| B2 `bug_b_error_status_or_error_object_is_invocation_failed` | 9956 | `Some(InvocationFailed)` | 通过（绿） |
| B3 `bug_b_ok_without_error_field_is_none` | 9992 | `None` | 通过（绿） |
| B4 `bug_b_ok_with_non_null_error_object_is_invocation_failed` | 10004 | `Some(InvocationFailed)` | 通过（绿） |

- B1 的 output 逐字段复刻 tools.rs run_command 成功路径形状（~L1942-1970）：
  `{"ok":true,...,"exitCode":0,"stdout":"hello\r\n","stderr":"","error":null,"summary":{...},"permission":{...}}`。
- B5（评审补测）`bug_b_ok_with_empty_string_error_is_none`：status="ok" 且 `"error": ""`（空串）→ `None`，
  与 tool_error_from_output 对 status="ok" 恒 None 的读侧语义一致。

## 红相/绿相演进记录
1. 红相锚定（commit 05e0576，修复前）：B1 红（实际 `Some(InvocationFailed)`，期望 `None`），B2/B3/B4 绿。
2. Executor 提交 `2b63b39 fix(runtime): classifier treats error:null/empty-string success as no failure`
   （error 为 null 或空串时视为无错误）后，本机 `cargo test --lib bug_b_`：B1-B5 全部通过（绿）。
   B5 在修复落地后为绿（实现已含空串跳过）。

## 运行命令
```bash
cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test --package pony-agent-core --lib bug_b_
```

## 输出片段（红相证据）
```
running 4 tests
test agent::runtime::tests::parity::bug_b_ok_without_error_field_is_none ... ok
test agent::runtime::tests::parity::bug_b_ok_with_non_null_error_object_is_invocation_failed ... ok
test agent::runtime::tests::parity::bug_b_error_status_or_error_object_is_invocation_failed ... ok
test agent::runtime::tests::parity::bug_b_ok_run_with_error_null_is_none_not_invocation_failed ... FAILED

---- ... bug_b_ok_run_with_error_null_is_none_not_invocation_failed stdout ----
thread '...' panicked at crates/pony-agent-core/src/agent/runtime/tests.rs:9948:9:
assertion `left == right` failed: status=ok 且 error:null 的 Run 成功结果不应被分类为失败（Bug B 红相）
  left: Some(InvocationFailed)
 right: None

failures:
    agent::runtime::tests::parity::bug_b_ok_run_with_error_null_is_none_not_invocation_failed

test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 975 filtered out
```

## 基线（加测试前）
`cargo test ... tool_result_failure_kind` → 0 个匹配测试（该函数此前无任何测试）。

## 阻塞/注意事项
- 完整 `cargo test`（含集成测试目标）被未跟踪文件 `crates/pony-agent-core/tests/ask_user_acceptance.rs`
  的编译错误阻断（E0599/E0716，HEAD 上即存在，非本次改动引入，未触碰）。
  故红相验证使用 `cargo test --lib` 限定单测目标。
- 修复方向（Executor 执行）：`tool_result_failure_kind` 对 `error` 为 JSON null 时应按"无 error"处理
  （B1 转绿），其余分支行为不变（B2/B3/B4 保持绿）。
