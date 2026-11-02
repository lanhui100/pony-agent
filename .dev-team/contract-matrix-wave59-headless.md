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

## 4. v4 裁定（针对 `review-plan-wave59.json` 的 9 项 BLOCKING，Lead 一票裁决）

| # | 审查项 | 裁定 | 落地条款 |
| :-- | :--- | :--- | :--- |
| B1 | 超时收敛不可兑现（worker 不杀 / exit 跳 Drop / 后台续写） | **接受，降级为「声明式收敛」** | 超时路径：落 envelope → **显式 drop observability guard 完成 flush** → `exit(7)`。**如实声明**：worker 线程不可抢占、可能仍写 session/trace；不承诺幂等清理。NFR 同步改写，禁称「worker 已关闭」。 |
| B2 | C1 stdout 纯净在写域内不可达 | **接受，扩写域** | 允许 Executor 改 `crates/pony-agent-core/src/agent/otel/log.rs`（≤20 行，新增输出目标开关，**默认行为不变**）。禁止 dup2 hack。 |
| B3 | suspended→6 无法区分 ask/approval | **接受为已知局限** | envelope 增 `failure_reason`（透传 wire 的 `control_outcome_pending` 原文）；区分两者需 wire 增字段 → PA-167。 |
| B4 | text 逐字节兼容 ⟂ C4 默认 deny | **接受，分模式默认** | `text` 模式未显式给 `--permission-mode` ⇒ 传 `None`（**向后兼容**）；`jsonl`/`sse` 模式默认 `ReadOnly`（fail-closed）。`HeadlessOptions::default()` 仍为 `ReadOnly`。 |
| B5 | workspace-write/full-access 与不传等价＝静默越权 | **接受，必须外显** | 非 `ReadOnly` 档位时 stderr 输出 `WARN permission-mode=<m> 与「不传」在运行时限内判定等价（已知局限）`；`--help` 同步标注。 |
| B6 | `run_headless(&HeadlessOptions)` 不可实现不可测 | **接受，删除** | 从契约删除该符号（与 `max_turns` 同理：无 turn 输入 ⇒ 死接口）。 |
| B7 | 冻结 sink 是 unit struct 无法缓冲 | **接受** | `JsonlSink`/`SseSink` 改为**带私有状态的具名结构体**（`new()` 签名不变，测试不受影响）。 |
| B8 | E2E 只 mock Read、无 `--timeout-secs`、仅断言非 0 ⇒ 可假绿 | **接受为 TEST-DEFECT（豁免计数）** | Test Agent 补：① `--timeout-secs 1` + 慢 mock ⇒ 断言 exit **7**；② suspended 覆盖（若无可控路径则显式标注 `[RED DEGRADED]` 并入 PA-167）；③ 加强断言到具体码值。改后重锚红相。 |
| B9 | worker panic 结局未定义 | **接受** | worker 内 `catch_unwind`；panic ⇒ `HeadlessOutcome::Failed` ⇒ exit ≠ 0。 |

ADVISORY 采纳：`TurnStatus` 加 `#[serde(rename_all="lowercase")]`（与 NFR 小写令牌一致）；NFR `contract_version` 升 v4；`external_call_timeout_ms` 注明「仅约束本波新增外部调用，不改既有 provider 180s 读预算」。

## 5. 变更流水
- 2026-11-02 v4：吸收 review-plan-wave59 的 9 项 BLOCKING 裁定（见 §4）；删 `run_headless`；sink 改具名结构体；text/jsonl 分模式权限默认；`failure_reason` 入 envelope；扩写域 `agent/otel/log.rs`。
- 2026-11-02 首版冻结（Lead）。
- 2026-11-02 v3：补 `impl Default`（默认 ReadOnly）；删除 `max_turns` 字段（语义空洞，转 PA-167）。
- 2026-11-02 v2：C4 收窄（撤销 allow-tools/decide，改 tool_authorization_mode 映射）；新增 Suspended 终态 → exit 6；C5 明确 worker 线程模型。依据：E1/E2 只读侦察。
