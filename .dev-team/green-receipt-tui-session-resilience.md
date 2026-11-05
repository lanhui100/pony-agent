# wave-tui-session-resilience 绿相收据

- worktree: `/home/dm/pony-agent-worktrees/tui-session-resilience`
- 红相锚定: `4f98062e` / tag `wave-1-tui-session-resilience-red`
- 执行者: Executor（L2-E）；未执行 git commit/push；未修改任何测试文件。
- 说明: `.dev-team/contract-matrix-tui-session-resilience.md` 的 diff 为 Protocol/Lead 在冻结后原地补录的口径裁决与归属，Executor 只读；`crates/pony-agent-core/tests/wave58_context_skills.rs` 的 1 行编译衍生同步由 Lead 完成并登记 derivative-scope 豁免。

## 门禁（原始输出见 /tmp/executor-*.log）

| 命令 | Exit | 原始 stdout 摘要 |
| :-- | --: | :-- |
| `cargo check -p pony-agent-core --all-targets --message-format short` | 0 | `Checking pony-agent-core v0.1.154`; `Finished \`dev\` profile [unoptimized] target(s) in 19.85s` (`/tmp/executor-check-final.log`) |
| `cargo test -p pony-agent-core --test tui_session_resilience_acceptance --message-format short` | 0 | `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s` (`/tmp/executor-final-tui_session_resilience_acceptance.log`) |
| `cargo test -p pony-agent-core --test tui_overlay_cancel_acceptance --message-format short` | 0 | `test result: ok. 19 passed; 0 failed; ... finished in 10.37s` (`/tmp/executor-final-tui_overlay_cancel_acceptance.log`) |
| `cargo test -p pony-agent-core --test tui_render_layout --message-format short` | 0 | `test result: ok. 12 passed; 0 failed; ... finished in 0.03s` (`/tmp/executor-final-tui_render_layout.log`) |
| `cargo test -p pony-agent-core --test tui_resume_cli --message-format short` | 0 | `test result: ok. 8 passed; 0 failed; ... finished in 0.00s` (`/tmp/executor-final-tui_resume_cli.log`) |
| `cargo test -p pony-agent-core --test tui_terminal_restore --message-format short` | 0 | `test result: ok. 8 passed; 0 failed; ... finished in 0.00s` (`/tmp/executor-final-tui_terminal_restore.log`) |
| `cargo test -p pony-agent-core --test pa104_terminal_acceptance --message-format short` | 0 | `test result: ok. 5 passed; 0 failed; ... finished in 0.19s` (`/tmp/executor-final-pa104_terminal_acceptance.log`) |
| `bash ~/.agents/skills/observability-onboarding/scripts/check-declaration.sh .` | 0 | `[PASS] metrics / tracing / logs / issues`; `结论: PASS (exit=0)` (`/tmp/executor-observability-declaration.log`) |
| ADR status 机械检查 | 0 | `grep -L '^Status:' ...` 与 `grep -L '^Status: implemented' docs/decisions/[0-9]*.md` 输出均为空 |

## 实现与契约映射

| 契约 | 文件:行 | 实现 |
| :-- | :-- | :-- |
| C1 | `provider/mod.rs:4330` / `openai_sse.rs:68,158,310,360` / `bin/tui.rs:293,3260` | DSML 块折叠为「工具调用」占位的单一 provider 纯函数；SSE delta、累计终态、TUI `turn:delta` 复用；半截标记在 `bin/tui.rs:268-291` 判定 `incomplete` 且不输出。 |
| C2 | `bin/tui.rs:7184,7222` | `hydrate_state_from_session` 的 user/assistant 历史回放共用 S1；旧会话无检查点文本逐字回放。 |
| C3/C4 | `bin/tui.rs:306,329,347,8784-8803` | 不新增 TUI 重试循环；瞬态错误显示「重试中（1/5）」简短状态，终态只显示一句话摘要；原始详情仅 `eprintln!` 到运行日志，issue 上报仍由既有 `capture_turn_failure` 承担。 |
| C5/C6 | `session/store.rs:64-90` / `session/mod.rs:17-20` / `runtime/turn_persist.rs:543-572` / `runtime/turn_control.rs:172-196` / `bin/tui.rs:365,395,7228-7258` | 中断轮次把已落盘工具活动投影为 additive 检查点文本写入既有历史；resume 识别未完成 turn 并接续展示，不重复已完成 turn；空输入/旧会话返回 `None`。 |
| C7/C8 | `bin/tui.rs:415,8940-8990,6717-6722` | `exit_cleanup_plan` 严格 `cancel→wait→restore→hint→exit`、超时钳 3000；事件循环退出先 `stop_turn` + 有界等待；`run_tui` 成功路径在 `restore` / `print_resume_hint` 后 `otel::shutdown()` + `process::exit(0)`。 |
| 归属/ADR | `docs/observability.yaml` 尾部 / `docs/decisions/0064-tui-session-resilience.md` / `docs/decisions/README.md` | 四维归属不变，仅登记；新增 implemented ADR 0064（含候选方案比较）。 |

## 未覆盖 / 局限

- PTY 动态验证：本次未拉起真实 `pony` 进程做退出后 `ps` / termios / `/dev/tty` 持有断言；冻结验收用例 C8 明确标注 `[RED DEGRADED: STATIC-ONLY]`，本收据不冒充动态通过。真实 PTY 冒烟留给 Lead 的 Join Barrier。
- `session/mod.rs` 的 3 行纯 re-export 由 Lead 裁决为 `lead-assembly` 白名单例外（无业务逻辑，避免检查点纯函数双真值源）。
- 未新增重试预算、网络端点、环境变量、依赖、持久化 schema 字段或外部异常码。
