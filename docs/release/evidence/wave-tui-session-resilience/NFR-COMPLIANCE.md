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

PTY 动态冒烟已由 pty-holding wrapper 打通：/quit 与 SIGTERM 双路径退出后同一 pts 的 termios 由 `-isig/-icanon/-echo/-opost` 复原为 `isig/icanon/echo/opost`（stty rc=0），负控制证明 wrapper 不自行修复；C8 动态 PASS，0 degraded。见 pty-smoke-receipt.json / pty-smoke-final-run.stdout / pty-smoke-negative-control.json。
