# wave-tui-session-resilience 冻结契约矩阵

基线 commit: `7a5269ed`（worktree `/home/dm/pony-agent-worktrees/tui-session-resilience`，分支 `fix/tui-session-resilience`）。
冻结时间：2026-11-06。契约唯一编写方：Protocol Agent（因超时由 Lead 依「空桩/契约文档」权限代冻结，业务实现仍归 Executor）。
本波为 B 级（跨文件、涉 UI 交互与外部 IO、命中并发/退出/重试关键字）⇒ NFR + 双模态红相 + L3-P 硬门禁。

## 0. 四维取证结论（resolve-issues，≥2 独立维度一致）

| 维度 | 证据 | 结论 |
| :-- | :-- | :-- |
| Issues | PonySentry `triage --env development --hours 72` exit 0 取回 32 条；全量 87 条中 `project=pony-agent` 的 `TurnExecutionFailed` 5 条：`42f32099…`(count=9, `http://127.0.0.1:1` connect)、`6aff22aa…`(timeout 45014ms)、`ab281e5f…`(missing API key)、`8a2feda6…`(工具连续失败 4 次)、`85fb442c…`(IpcCommandError updater) | 错误路径**有** issue 上报；用户所见「持续错误」与 provider 连接/超时类失败同族 |
| Traces | `rca.py trace 8d2ad36237874187897975d87701a329` exit 0，`agent.turn` STATUS_CODE_ERROR；`6c7e7a65…` 同 | turn 失败链路有 trace，错误止于 agent.turn 根 span |
| Logs | 本地 `~/.pony-agent/tui-logs/tui-runtime.log` 329046 行：`decision:attempt=1/5 failed class=NonRetryable 401`、`emit turn:failed` 18 次、`[pony-tui] running terminal restore (T2c)` 742 次且其后仍有 provider usage | 错误详情直显 + 退出后后台请求继续 |
| Metrics | `docs/observability.yaml` metrics 结构性 N-A（桌面/TUI 无常驻 /metrics）；`rca.py plane --env dev` Loki 覆盖不可证 exit 2（结构性 N-A 一致） | 不以 metrics 作为结案依据，显式标注缺维 |
| 会话库 | `~/.local/share/PonyAgent/sessions.db` `normalized_messages` rowid 166/184（session-83e412b0）content 含完整 `<｜｜DSML｜｜ calls> ... <｜｜DSML｜｜ parameter name="description">` | DSML 原文已进入持久化历史 |
| 进程 | `ps` PID 3648409 `pony` pts/0 ELAPSED>2h；`stty -F /dev/pts/0 -a` = `-isig -icanon -echo -opost`；22 线程（otel/ponysentry/tokio/trace-persist）；fd0/1=/dev/pts/0、fd13=/dev/tty、tpgid=自身 | 退出后进程与 raw mode 一起残留 |

G1 交叉：Issues(错误有上报) × Logs(错误详情直显/退出后请求继续) × 会话库(DSML 持久化) × 进程(raw mode 残留) ≥2 维一致 ⇒ 根因结论成立；Logs 维为本地取证面（Loki 结构性 N-A，已显式说明）。G5 变更相关性：最近 `fix(tui)` 8d18d157 只改 sink 生命周期/结构化载荷抑制，未触及本波四条链路。

## 1. 归属判定

- 用户「本项目中修正 tui 的会话」= pony-agent 的 Rust TUI（`crates/pony-agent-core/src/bin/tui.rs`），非 DSH GUI。证据：DSH checkout HEAD `de5d733a18` 已删除 TUI（`10bb9cbf4a cleanup: remove TUI package`），全仓无 alternate screen/raw mode 源码；用户贴的 DSML 命令含 `composer_enter_action`（pony-agent tui.rs:396 专有符号）。
- DSML 标记生成方为上游模型/网关（两仓库源码均无 DSML 常量）[UNVERIFIED 生成方]；本项目侧的确证缺陷是「收到后原样渲染 + 原样持久化 + 回灌 provider」。

## 2. 冻结纯函数签名（红相唯一可编译接缝）

全部位于 `crates/pony-agent-core/src/bin/tui.rs` 顶层 `pub`，空桩体为 `unimplemented!()`；测试经 `#[path = "../src/bin/tui.rs"] mod tui;` 可达。Executor **只允许替换函数体，不得改名/改签名/删除/新增依赖**。

| 编号 | 签名 | 语义 |
| :-- | :-- | :-- |
| S1 | `pub fn sanitize_model_protocol_text(text: &str) -> (String, bool)` | C1/C2；流式与历史回放共用；返回清洗后文本 + 尾部是否疑似被 chunk 切断的协议片段 |
| S2 | `pub fn is_transient_provider_error(error: &str) -> bool` | C3；仅 timeout/rate-limit/5xx/network 类为 true；401/403/400/missing key 为 false |
| S3 | `pub fn user_facing_error_summary(error: &str) -> String` | C4；一句话摘要，不含 400 字符响应体/堆栈 |
| S4 | `pub fn should_fold_error_detail(error: &str) -> bool` | C4；终态错误默认折叠详情 |
| S5 | `pub fn interrupted_turn_checkpoint(prior_assistant_text: &str, tool_activities: &[String]) -> Option<String>` | C5/C6；空输入返回 None（旧会话向后兼容） |
| S6 | `pub fn resume_from_checkpoint(completed_history: &str, checkpoint: Option<&str>) -> String` | C6；不重复已完成 turn；无检查点时原样返回 |
| S7 | `pub fn exit_cleanup_plan(inflight_turn: bool, timeout_ms: u64) -> Vec<&'static str>` | C7/C8；顺序 `cancel → wait → restore → hint → exit`；timeout_ms ≤ NFR 3000 |

## 3. 契约断言（原地冻结，禁追加新章节）

| ID | 现象/根因证据（worktree path:line） | 验收断言 | 写域 |
| :-- | :-- | :-- | :-- |
| C1 | `openai_sse.rs:457` delta.content 原样文本 → `tui.rs:3091` `TuiTurnSink::emit` → `tui.rs:2619` `append_assistant_chunk` push_str | `sanitize_model_protocol_text` 对完整 `<｜｜DSML｜｜ calls>…invoke…parameter…</｜｜DSML｜｜ calls>` 返回不含任何 `｜DSML｜｜`/`invoke name=`/`parameter name=` 的文本且含「工具调用」占位；流式跨 chunk 分片时 `incomplete=true` 且不得输出半截标记；普通 markdown 代码块/含尖括号正文逐字保真 | tui.rs / openai_sse.rs |
| C2 | `tui.rs:6962` `hydrate_state_from_session` 回放 `hist.content.clone()` 无过滤；sessions.db rowid 166/184 已存 DSML | 历史回放经 S1 清洗；旧会话无检查点字段可加载；清洗不破坏 user/assistant 角色序列 | tui.rs / store.rs |
| C3 | `provider/mod.rs:728/811` `retry_provider_timeout` 已存在 5 次/30s 预算；`retry.rs:601` classify；日志 `decision:attempt=1/5 … 401 NonRetryable` | S2 对 401/403/400/missing key 返回 false、对 timeout/429/5xx/network 返回 true；不得新增 TUI 层重试循环（防风暴）；重试期状态栏显示「重试中（n/N）」而非裸详情 | provider/mod.rs / tui.rs |
| C4 | `provider/mod.rs:4403` `extract_provider_error_detail` 截 400/240 字符；`tui.rs:8559` `status_text="Turn failed: {err}"` + System `Error: {err}` | S3 摘要长度有界且不含 `error sending request for url`/JSON 响应体/堆栈；S4 终态 true；原始详情仅留运行日志与 issue 上报，不铺消息区 | tui.rs / provider/mod.rs |
| C5 | `turn_control.rs:172` cancel 写死占位「用户终止，发送消息可继续。」；`store.rs:417` `append_turn` 无 turn-scoped events 时 fallback caller text | S5 在非空文本/工具活动时返回 Some(检查点)；空输入 None；检查点持久化格式 additive（旧会话无字段可加载） | turn_persist.rs / turn_control.rs / store.rs / sqlite_session.rs |
| C6 | `tui.rs:6962` resume 只回放 `session_snapshot.history`；日志 `observation ref backfill skipped: no trace row` | S6 有检查点时在其后接续、不重复 `completed_history`；无检查点时逐字返回；中断 turn 不被整轮撤销 | tui.rs / session 层 |
| C7 | `tui.rs:176` 信号处理器先 `restore_terminal_for_signal()` 再置 `SHUTDOWN_REQUESTED`；`tui.rs:7992` 主循环 tick break；`tui.rs:8381/8426` `start_turn_stream` 在 `thread::spawn` 内；`control_plane/mod.rs:2089` `stop_turn` | S7 在 `inflight_turn=true` 时首步为 `cancel` 且顺序固定；`timeout_ms>3000` 钳到 3000；退出路径调用 `stop_turn` + 有界 join + `restore_terminal_modes` + `print_resume_hint` 后 `process::exit` | tui.rs / control_plane 既有接缝 |
| C8 | `stty /dev/pts/0`: `-isig -icanon -echo -opost`；22 线程；742 次 restore 后仍有 provider usage | PTY 断言：退出后进程不存在、`LeaveAlternateScreen`/`TERMINAL_SCREEN_RESET` 字节发出、termios 复原（`-echo` 消失）、`pony resume <id>` 在还原后输出；无活线程持有 /dev/tty | tui.rs / main |

## 4. 写域与不变量

- Test 写域：`crates/pony-agent-core/tests/tui_session_resilience_acceptance.rs`、`tests/acceptance/adversarial/tui_session_resilience/**`、`.dev-team/red-phase-tui-session-resilience.json`、`.dev-team/l2at-done-tui-session-resilience.json`。
- Executor 写域：`src/bin/tui.rs`、`src/agent/provider/{openai_sse,mod}.rs`、`src/agent/{sqlite_session.rs,session/{store,backend,types}.rs}`、`src/agent/runtime/{turn_persist,turn_control,turn_stream,turn_sync}.rs`、`docs/observability.yaml`、`.dev-team/green-receipt-tui-session-resilience.md`。**禁改任何测试**。
- 公共契约不变量：不新增网络端点/环境变量/依赖/异常码；不新增 Loki/metrics 出口；issue 上报仍走 `capture_turn_failure`；`pony resume <id>` 命令形状不变；持久化 additive-only。
- 可观测性归属：四维不变（tracing/issues applicable，metrics/logs 结构性 N-A），仅 `docs/observability.yaml` 追加归属登记；验收需 `audit-diff.sh` + `check-declaration.sh` Exit 0/WARN 明示。

## 5. 门禁

红相：`cargo test -p pony-agent-core --test tui_session_resilience_acceptance` 必须以 assert/`unimplemented!()` 原生失败（非 E0425）→ Lead 锚定 commit。
绿相：`cargo check --all-targets`、验收测试、回归 `tui_overlay_cancel_acceptance` / `tui_render_layout` / `tui_resume_cli` / `tui_terminal_restore` / `pa104_terminal_acceptance` 全 Exit 0。
Join Barrier：写域审计、blast-radius、NFR 数值比对、review-lint、L3-P 六问、PTY 冒烟、可观测性变更门禁、元复盘。

## 变更流水

- 2026-11-06 Lead 代冻结：7 个纯函数空桩 + NFR + 本矩阵；`cargo check --all-targets` Exit 0；`wave58_context_skills.rs:567` 1 行编译衍生同步登记为 derivative-scope 豁免。
