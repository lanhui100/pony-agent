# 契约矩阵 · wave-59 · PA-164 Headless 交付形态补全

> 基线 commit：`a0d6232`（worktree `/home/dm/wt/pony-wave59`，分支 `wave-59/headless`）
> 冻结人：Lead。本表为唯一真值源；变更须走契约演进门禁（Protocol 出新版 + 重锚红相）。
> 立项卡：`management/task-system/03_TASKS/PA-164-headless-delivery-form.md`

## 0. 写域与角色

| 角色 | 写域 | 禁止 |
| :--- | :--- | :--- |
| Protocol (L2-P) | `src/agent/headless.rs`(空桩)、`src/agent/mod.rs`、`.dev-team/nfr-baseline.json` | 业务逻辑、测试、`bin/cli.rs` |
| Test (L2-T) | `tests/headless_contract.rs`、`tests/acceptance/wave59/` | 一切 `src/` |
| Executor (L2-E) | `src/agent/headless.rs`(实现)、`src/bin/cli.rs` | 验收测试（冻结只读） |
| Lead | 契约矩阵、git 写、join barrier、收口 | 业务代码 >50 行 |

## 1. 冻结契约（每条一断言）

- **C1 输出模式**：`--output-format text|jsonl|sse`，默认 `text`。`text` 模式与基线逐字节兼容；`jsonl`/`sse` 下 **stdout 只承载事件**，横幅/诊断一律转 stderr；每行 JSON 可 `serde_json::from_str`，**不得含 ANSI 转义**（`\x1b`）。
- **C2 退出码**：`0`Success / `1`Internal / `2`Usage / `3`Config / `4`ProviderNetwork / `5`ToolFailure / `6`PermissionDenied / `7`Timeout。**fail-closed**：`classify_failure` 未知错误 → `Internal(1)`，严禁兜底 `Success`；`turn:failed` 绝不退 0；分类用穷举 match。
- **C3 结果封套**：`--result-json <path>` 落 `ResultEnvelope{schema="pony.headless.result.v1", session_id, turn_id, status, exit_code, final_text, usage{input_tokens,output_tokens}, tool_calls[{name,status}], duration_ms}`；原子写（tmp+rename）；`status==Completed ⟺ exit_code==0`，`status==Failed ⟹ exit_code!=0`。
- **C4 审批策略（v2 收窄）**：`--permission-mode deny|workspace-write|full-access`，**默认 `deny`**，1:1 映射到既有 `TurnInput.tool_authorization_mode`（`read_only`/`workspace_write`/`full_access`，真值源 `dispatcher.rs:73`、`governed_executor.rs:310-333`）。
  **撤销 `--allow-tools` 与 `PermissionMode::decide()`**：宿主无「可注入策略求值接缝」，自造放行判定而不被 runtime 消费 = 静默谎报（PA-131 教训）。缺口登记 PA-167。
  **`turn:suspended` 是终态**（`control_plane/mod.rs:1956-1975`；`turn_stream.rs:376-438` 只发 `turn:suspended` 即返回）：无头宿主**无任何审批应答方** ⇒ 必须收敛为 `ExitCode::PermissionDenied(6)`，**禁止**当运行中（否则 CLI 死循环挂死，见 E2 侦察）。
- **C5 有界执行**（v2，`--max-turns` 已删）：`start_turn_stream` 同步阻塞（`control_plane/mod.rs:1931`）⇒ worker 线程跑 runner + 调用线程 `recv_timeout`；`--timeout-secs <n>` 到期 → `status=Timeout` + exit `7`；**`--max-turns` 撤销**（单发 CLI 恒为 1 轮＝语义空洞；工具跳数上限在 `runtime/limits.rs:76` 属 `pub(super)` 无宿主接线口）⇒ 与 `--allow-tools` 一并登记 PA-167。`HeadlessOptions::default().permission_mode == ReadOnly`（fail-closed）。
- **C6 验收**：`cargo test -p pony-agent-core --test headless_contract` 全绿；`cli_acceptance` 不回归；`RUSTFLAGS="-D warnings" cargo check -p pony-agent-core` Exit 0；真实二进制 E2E 落盘 exit code + stdout 纯净性证据。

## 2. 非目标
不改 TUI（`bin/tui.rs`）与 Tauri（`src-tauri/`）行为；不实现 ACP（PA-165）/ SDK（PA-166）；不重构 REPL 交互语义。

## 3. 变更流水
- 2026-11-02 首版冻结（Lead）。
- 2026-11-02 v3：补 `impl Default`（默认 ReadOnly）；删除 `max_turns` 字段（语义空洞，转 PA-167）。
- 2026-11-02 v2：C4 收窄（撤销 allow-tools/decide，改 tool_authorization_mode 映射）；新增 Suspended 终态 → exit 6；C5 明确 worker 线程模型。依据：E1/E2 只读侦察。
