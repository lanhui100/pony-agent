# NFR 达标报告 · wave-tui-session-resilience

| 基准字段 | 实现值 | 阈值 | 代码/机器证据 | 结论 |
| :-- | :-- | :-- | :-- | :-- |
| external_call_timeout_ms | 3000 | <=3000 | bin/tui.rs exit_cleanup_plan + wait deadline 3000ms | PASS |
| retry.max_attempts | 5 | <=5 | provider/mod.rs PROVIDER_TIMEOUT_RETRY_MAX_ATTEMPTS=5 unchanged; TUI no new loop | PASS |
| retry.backoff_cap_ms | 8000 | <=8000 | provider retry.rs unchanged | PASS |
| concurrency_lock | SHUTDOWN_REQUESTED AtomicBool + control_plane.stop_turn | single writer | SHUTDOWN_REQUESTED + stop_turn; AT2/AT3 deterministic | PASS |
| logging.format | json | json | existing PONY_TUI_RUNTIME_LOG; no new sink | PASS |
| logging.trace_id | True | true | unchanged capture_turn_failure/OTel | PASS |
| resource_release | 3 | 3 items | stop_turn/join, restore_terminal_modes, otel::shutdown + process::exit | PASS |

机器收据：cargo check --all-targets EXIT 0；RUSTFLAGS=-D warnings EXIT 0；验收 15/15；回归 52/52；audit-diff EXIT 0；check-declaration EXIT 0。

局限：PTY termios 读回因 tmux 销毁 pts 不可测（DEGRADED），其余 C8 动态断言通过，见 pty-smoke-lead-adjudication.json。
