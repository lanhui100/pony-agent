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
| `bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base 7a5269ed --repo . --decl-approved` | 0 | `[OK] audit-diff: 无越界实现、收口点完整`；D1 声明改动在 89bce8c2，显式批准，非削弱（四维归属不变，仅追加归属登记） |
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

- PTY 动态验证：Join Barrier 已由 Lead + PTY 冒烟执行者用 pty-holding wrapper 完成（`docs/release/evidence/wave-tui-session-resilience/pty-smoke-receipt.json`）：/quit 与 SIGTERM 双路径进程退出、termios 由 raw 复原（stty rc=0）、`pony resume <id>` 提示可见、负控制证明 wrapper 不自行修复；VERDICT=PASS，0 failed / 0 degraded。冻结验收用例 C8 的静态标注保留为测试层事实，不冒充动态；动态证据另计。
- `session/mod.rs` 的 3 行纯 re-export 由 Lead 裁决为 `lead-assembly` 白名单例外（无业务逻辑，避免检查点纯函数双真值源）。
- 未新增重试预算、网络端点、环境变量、依赖、持久化 schema 字段或外部异常码。

## 补丁绿相（wave-2 分片闭合）

- 触发：L3 反例 `/tmp/l3a-repro/repro`：完整 DSML 块切成 `["<｜","｜DSML｜｜ calls>…"]` 逐片清洗累加得 `"<｜［工具调用］"`，provider `output_text` 残留半截协议前缀。
- 改动（唯一）：`crates/pony-agent-core/src/agent/provider/mod.rs:4330-4405`，`sanitize_dsml_protocol_text` 新增 `dsml_partial_tail_len` 尾部半截前缀识别（合法 UTF-8 边界、已知标签前缀、尾部有限窗口），函数体改对 `body` 扫描并在收尾二次截断；孤立 `｜DSML｜｜` 行仍整行剔除。未改签名、未改测试、未改其他文件、未 git 写。
- 防误伤：单个 `<` 结尾正文与 markdown 代码块逐字保真（CHUNK-A02 绿）。

| 命令 | Exit | 原始 stdout 摘要 |
| :-- | --: | :-- |
| `cargo test -p pony-agent-core --test tui_session_resilience_chunk_split --message-format short` | 0 | `test c1_chunk_split_dsml_prefix_is_not_persisted ... ok`; `test c1_plain_angle_bracket_text_is_verbatim ... ok`; `test result: ok. 2 passed; 0 failed` (`/tmp/executor-patch-final-tui_session_resilience_chunk_split.log`) |
| `cargo test -p pony-agent-core --test tui_session_resilience_acceptance --message-format short` | 0 | `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s` (`/tmp/executor-patch-final-tui_session_resilience_acceptance.log`) |
| `cargo check -p pony-agent-core --all-targets --message-format short` | 0 | `Checking pony-agent-core v0.1.154`; `Finished \`dev\` profile [unoptimized] target(s) in 57.50s` (`/tmp/executor-patch-final-check.log`) |

## Wave-3 绿相（L3-B 遗留反例）

- 红相：`crates/pony-agent-core/tests/tui_session_resilience_l3b.rs` 4/4 原生 assert 失败（tag `wave-3-tui-session-resilience-red`）。
- 改动（4 文件、最小闭包；未改测试/签名/git 写）：
  - `crates/pony-agent-core/src/agent/provider/mod.rs:4323-4405`：`sanitize_dsml_protocol_text` 新增 `dsml_signature`（小写归一 + 全角/半角竖线统一）、`is_dsml_protocol_line` 与变体感知的 `dsml_partial_tail_len`；协议行整行折叠为「工具调用」占位，合法 UTF-8 边界、行首/换行约束保证普通 `<` / 半角文本 / markdown 代码块逐字保真。
  - `crates/pony-agent-core/src/agent/context.rs:5-8,1855-1890`（追加授权写域）：`to_provider_history_message` 与 `history_to_native_messages` 复用同一清洗纯函数，阻断本机存量 DSML 历史回灌 provider 的两路投影。
  - `crates/pony-agent-core/src/bin/tui.rs:7263-7284`：message_state fallback 的 user/assistant `entry.content` 经 `sanitize_model_protocol_text`。
  - `crates/pony-agent-core/src/agent/session/store.rs:4006-4018`：`classify_turn_node_kind` 对含 `[中断轮次检查点]` 的失败轮保持 `TurnCancelled`，精确取消文案控制组语义不变。

| 命令 | Exit | 原始 stdout 摘要 |
| :-- | --: | :-- |
| `cargo test -p pony-agent-core --test tui_session_resilience_l3b --message-format short` | 0 | `running 4 tests`; 4 条 L3B-A01..A04 全 `ok`; `test result: ok. 4 passed; 0 failed` (`/tmp/executor-w3-final2-tui_session_resilience_l3b.log`) |
| `cargo test -p pony-agent-core --test tui_session_resilience_acceptance --message-format short` | 0 | `test result: ok. 15 passed; 0 failed` (`/tmp/executor-w3-final2-tui_session_resilience_acceptance.log`) |
| `cargo test -p pony-agent-core --test tui_session_resilience_chunk_split --message-format short` | 0 | `test result: ok. 2 passed; 0 failed` (`/tmp/executor-w3-final2-tui_session_resilience_chunk_split.log`) |
| `cargo test -p pony-agent-core --test tui_overlay_cancel_acceptance --message-format short` | 0 | `test result: ok. 19 passed; 0 failed` (`/tmp/executor-w3-final2-tui_overlay_cancel_acceptance.log`) |
| `cargo test -p pony-agent-core --test tui_render_layout --message-format short` | 0 | `test result: ok. 12 passed; 0 failed` (`/tmp/executor-w3-final2-tui_render_layout.log`) |
| `cargo test -p pony-agent-core --test tui_resume_cli --message-format short` | 0 | `test result: ok. 8 passed; 0 failed` (`/tmp/executor-w3-final2-tui_resume_cli.log`) |
| `cargo test -p pony-agent-core --test tui_terminal_restore --message-format short` | 0 | `test result: ok. 8 passed; 0 failed` (`/tmp/executor-w3-final2-tui_terminal_restore.log`) |
| `cargo test -p pony-agent-core --test pa104_terminal_acceptance --message-format short` | 0 | `test result: ok. 5 passed; 0 failed` (`/tmp/executor-w3-final2-pa104_terminal_acceptance.log`) |
| `cargo check -p pony-agent-core --all-targets --message-format short` | 0 | `Finished \`dev\` profile [unoptimized] target(s) in 0.29s` (`/tmp/executor-w3-final2-check.log`) |

## Wave-3 绿相（L3-B 对抗反例闭合）

红相：`tui_session_resilience_l3b` EXIT 101，4/4 原生 assert 失败（A1 变体 / A2 历史回灌 / A3 fallback / A4 TurnCommitted 漂移）；dev-red-purity PASS。
实现：`provider/mod.rs` 归一化变体感知清洗；`context.rs` 两路历史入 provider 前清洗；`tui.rs` message_state fallback 清洗；`session/store.rs` 检查点失败轮保持 TurnCancelled。
Lead 独立全量门禁：`cargo check --all-targets` 0、`-D warnings` 0、l3b 4/4、chunk_split 2/2、主验收 15/15、回归 19+12+8+8+5=52/52、audit-diff --decl-approved 0、check-declaration 0。
真实 crate 探针：代码块内 DSML 标记被剔除且正文保留；嵌套/未闭合/凭据串不外泄；临时探针 `tests/tmp_w3_probe.rs` 已删除。
