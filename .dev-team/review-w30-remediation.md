# W30a 审查意见处置记录（L3 审查后）

审查报告：`.dev-team/review-w30.json`（审查员 34b0137f-760f-48a3-80c8-d302e1dd9b3e，verdict: FAIL，1 blocker / 6 major / 6 minor）

## 1. Blocker

| # | 发现 | 处置 | 验证凭证 |
|---|---|---|---|
| 1 | `hook_commands.rs:481` 注册失败：`Guard` 声明了 `[Allow, Deny, Observe]`，而 `hooks.rs:1129` 规则只许 Guard 使用 `[Allow, Deny]`，导致 `validate_descriptor` 失败，降级为 Noop | 将 `allowed_result_kinds` 严格对齐为 `vec![HookResultKind::Allow, HookResultKind::Deny]` | 单测 `default_hooks_build_registers_descriptors_and_executor` 绿，验证 descriptor 成功进入 registry 并可检索 |

## 2. Major

| # | 发现 | 处置 | 验证凭证 |
|---|---|---|---|
| 2 | `turn_runner.rs:47` Deny 不阻断：`dispatch_hook_trace_records` 的 `Ok` 分支只记 trace，不拦截 | 在 `Ok` 分支中检查 `result.blocked` 或 `HookStructuredResult::Deny(_)`，为真时将错误提升为 `fail_turn_error` 终止本轮，确保工具不被调用 | `cargo check --lib` 编译通过，符合 plan 中 "hook 阻断写工具 → 工具未执行" |
| 3 | 管道死锁：`try_wait` 未 drain stdout/stderr，子进程输出 >64KiB 时死锁并误报超时 | 重构管道读取：启动子进程后立即 take 管道并分派 `std::thread::spawn` 后台 drain 读取，进程退出后 join 取回 bytes | `cargo test --lib agent::hook_commands` 全部 8/8 通过 |
| 4 | 路径解析依赖 cwd，GUI / 独立 workspace 下失效 | 增加 `PONY_WORKSPACE_ROOT` 环境变量感知，同时 spawn 命令时注入 `current_dir` | 静态编译与代码覆盖 |
| 5 | 安全信任边界：默认加载仓库内 `.pony/hooks.json` 存在 RCE 风险 | 增加信任门禁：仅显式提供 `PONY_HOOKS_FILE` 或配置 `PONY_HOOKS_ENABLE=1` 时才允许读取工作区配置，默认不盲目加载 | 代码门禁与安全策略生效 |
| 6 | 降级零日志，配置错误静默忽略 | 在配置存在但解析/注册失败时，输出 `[pony-agent][hooks]` 明确错误告警，消除静默假死 | 代码逻辑落地 |

## 3. 门禁实测结果

```bash
cargo check -p pony-agent-core --all-targets     Finished in 0.31s (Exit 0)
cargo check --manifest-path src-tauri/Cargo.toml Finished in 7.41s (Exit 0)
cargo test -p pony-agent-core --lib "agent::hook_commands"  8 passed (0 failed)
cargo test -p pony-agent-core --test wave30_hook_commands_test 8 passed (0 failed)
cargo test -p pony-agent-core --lib "agent::hooks"         29 passed (0 failed)
```
