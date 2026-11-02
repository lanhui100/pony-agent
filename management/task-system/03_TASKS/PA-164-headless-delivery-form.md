# PA-164: Headless 交付形态补全（无头批处理 / CI 门禁形态）

## 状态
- **状态**: Review（wave-59 实现 + 双路红相 + 三路 Join Barrier 审查 + 收口门禁全部完成，待并入主干）
- **优先级**: P1
- **Complexity**: B
- **创建日期**: 2026-11-02
- **实施日期**: 2026-11-02（wave-59，分支 `wave-59/headless`，worktree `/home/dm/wt/pony-wave59`）
- **交付区间**: `056f8df`(契约空桩) → `20998d1`/`fee5e31`(双轮红相) → `9706859`(测试缺陷修复) → `46cc067`(实现) → `e8701a0`(收口)
- **前置依赖**: 无（**建议作为 PA-165 ACP / PA-166 SDK 的前置卡**）
- **宿主形态矩阵定位**: 四种交付形态（TUI / Headless / SDK / ACP）中的第 2 种，当前仅"部分具备"。

## 需求背景（物理事实，非推断）
现状二进制 `ponycli`（`crates/pony-agent-core/src/bin/cli.rs`，608 行）已具备无头调用的**骨架**：

- 参数面：`-p/--prompt`、`-m/--model`、`-g/--gateway`、`-w/--workspace`、`-c/--continue`、`-r/--resume <id>`、`--key`、`init` 子命令、stdin 管道合流（`read_stdin_if_piped`，:306）。
- 事件面：`TerminalSink`（:234-273）实现 `TurnEventSink`，已消费 `turn:delta / turn:tool_call / turn:tool_result / turn:completed / turn:failed / turn:output_end`。
- 可复用资产（**这是本卡成本低的关键**）：`agent/sse_adapter.rs`（290 行）已有 `format_sse_event`、`BufferingSseTurnEventSink`、`collect_turn_stream_frames`；`bin/non_tauri_harness.rs` 已有 `RecordingSink` 先例。

但它**不满足 Headless 形态的任一硬要求**，缺口全部可定位：

| 缺口 | 物理证据 |
| :--- | :--- |
| **无机器可读事件流** | `TerminalSink` 只输出带 ANSI 转义的人类可读文本（`:249` `\x1b[33m⚡ [Tool Call]\x1b[0m`），无 JSONL / SSE 选项。 |
| **退出码恒为 0** | `--prompt` 路径 `run_turn_interactive` 后直接 `return`（`:546-549`），`turn:failed` 只 `eprintln!`（`:263`）不传播；仅参数错误（`:498`）与 `init` 失败（`:505`）退 1。CI 无法据此判红。 |
| **无结果封套** | 无 `session_id / turn_id / status / usage / final_text` 的终态机器可读落点，调用方须抓 stdout 文本反解析。 |
| **stdout 被横幅污染** | `:510-513`「=== Pony Agent CLI ===」+ Model/Gateway/Workspace 无条件写 stdout，与结构化输出互斥。 |
| **无审批策略位** | `TurnInput.tool_authorization_mode` 恒为 `None`（`:366`）；`ask_user` 在无头下无应答方，行为未定义（潜在**挂起**）。 |
| **无执行上界** | 无 `--timeout`、无 `--max-turns`；provider 挂起或工具长跑时进程无限期存活。 |
| **无验收与 CI 接线** | `package.json` 只有 `"tui": "pony"`，无 headless script；`tests/acceptance/` 无 headless 真实进程套件。 |

## 目标
把 `ponycli` 从「人能看懂的交互式 CLI」升级为「**CI / 自动化可依赖的无头执行器**」：一条命令进、结构化事件出、退出码可判定、终态可审计、执行有界。

## 交付契约
- **C1 事件流契约**：`--output-format text|jsonl|sse`（默认 `text` 逐字节向后兼容）。
  `jsonl` 每行一条 `{"seq":n,"ts":"RFC3339","type":"turn:delta","payload":{...}}`；`sse` 复用 `format_sse_event`（不新造帧格式）。
  `jsonl`/`sse` 模式下 **stdout 仅承载事件**，横幅/诊断/日志一律转 stderr。
- **C2 退出码分类学（fail-closed）**：`0` 成功终态；`2` 参数错误；`3` 配置/凭据缺失；`4` provider 网络失败；`5` 工具执行失败；`6` 权限/审批拒绝；`7` 超时或预算耗尽；`1` 未知内部错误。
  **硬约束**：收到 `turn:failed` 不得退出 0；分类映射用**穷举 match**（新增错误类不落 `_ => 0` 兜底）。
- **C3 结果封套**：`--result-json <path>` 落终态唯一信封
  `{session_id,turn_id,status,exit_code,final_text,usage{tokens_in,tokens_out},tool_calls[{name,status}],duration_ms}`；
  信封与事件流描述同一终态（可对拍，不得二义）。
- **C4 非交互审批策略**：`--permission-mode {deny|allow-list|yolo}`，**默认 `deny`**（fail-closed）；
  `allow-list` 读显式工具白名单；`ask_user` / 待审批请求在无头模式下**必须确定性收敛**（按策略拒绝或应答），**禁止挂起**。
  落地方式复用既有 `TurnInput.tool_authorization_mode` 通道，不新增旁路。
- **C5 有界执行**：`--timeout-secs <n>`（整轮上界）、`--max-turns <n>`；超时按「SIGTERM → 宽限期 → 强杀进程树」收敛，退出码 `7`，且落 C3 信封 `status=timeout`。
- **C6 验收与接线**：新增 `tests/acceptance/l2at-headless.sh`（真实进程 + mock provider，断言退出码矩阵、JSONL 可解析性、stdout 纯净性、超时收敛）；
  `package.json` 增 `"headless": "ponycli"`；`docs/architecture/` 增补「四形态宿主矩阵」小节。

## 非目标
- 不实现 ACP（PA-165）与跨进程 SDK（PA-166）。
- 不改 TUI（`bin/tui.rs`）与 Tauri 宿主行为。
- 不重构 `cli.rs` 现有交互式 REPL 语义（仅在其上叠加输出模式与退出码）。

## 验收标准（物理收据）
1. `cargo test -p pony-agent-core --test wave_headless_contract` 全绿，含：退出码 7 档穷举、`turn:failed → exit≠0`、JSONL 每行可 `serde_json::from_str`、stdout 无 ANSI 泄漏。
2. `tests/acceptance/l2at-headless.sh` 落盘 `receipts/`：真实 Exit Code、bytes 级 stdout/stderr 分离证据、超时场景 `SIGTERM`+强杀日志。
3. `RUSTFLAGS="-D warnings" cargo check --workspace` EXIT 0；全量 `--lib` 无新增红相（对拍基线）。
4. `ponycli --output-format jsonl -p "hi" | jq -e .` 逐行通过（真实二进制）。


## 验收收据（物理，Lead 亲跑）

| 门禁 | 结果 |
| :-- | :-- |
| `cargo test -p pony-agent-core --test headless_contract` | **26 passed / exit 0**（红相 3/22 → 26/26） |
| `cargo test -p pony-agent-core --test cli_acceptance` | **5 passed / exit 0**（无回归） |
| `cargo test -p pony-agent-core --lib` | **1195 passed / exit 0**（与改前基线一致） |
| `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core` | **exit 0** |
| `bash tests/acceptance/wave59/headless-cli-e2e.sh` | **PASS=30 / FAIL=0 / exit 0**（红相 13/10 → 30/0） |
| `audit-diff.sh --base 9706859` / `check-declaration.sh` | exit 0 / PASS |

冒烟：`--help` exit 0；jsonl stdout 逐行合法 JSON、0 个 ANSI；SSE 4/4 帧合法；jsonl/sse 无 `-p` ⇒ exit 2；`--timeout-secs 999999` ⇒ exit 2；`--result-json` 遇符号链接不跟随、mode 600、无 tmp 残留。

**契约收敛记录（v1→v5）**：撤销 `--allow-tools`（宿主无可注入策略求值接缝）、`--max-turns`（单发恒 1 轮，跳数上限 `pub(super)` 无接线口）、`run_headless`（无 turn 输入的死接口）→ 全部登记 `PA-167`。新增 `turn:suspended → exit 6`（fail-closed）、`failure_reason` 透传、`JsonlSink`/`SseSink` 具名结构体、`TurnStatus` serde lowercase。

**声明式限制（不得声称已解决）**：超时后 worker 不可抢占、可能仍写 session/trace；stdout 背压（消费者停读）不在有界执行承诺内（POSIX 语义）。

**已知缺口**：`ponycli` 无 `--version`（HEAD 起即无，非本波回归）；E2E 无 sse 用例（由 Lead 冒烟补位）。

## 遗留登记
- `PA-167`：工具级授权/交互应答/跳数预算/`host_control_available` 注入（本波撤销项承接）。

## 当前进展
wave-59 已交付并收口（见上表）。

## 下一步动作
1. 按 `.agents/skills/dev-team` B 级路径立项：先出 spec（C1~C6 逐条可测）+ 红相锚定失败用例，再实施。
2. 先做 C1+C2+C3（成本集中在此，约占总工作量 60%），C4/C5 其次，C6 收尾。
3. 完成后同步 `docs/architecture/` 与 ADR（宿主形态矩阵 + 退出码契约）。

## 当前卡点
无技术阻塞。唯一决策点：**C2 退出码编号是否对外承诺为稳定契约**（建议承诺，并写入 ADR，否则 CI 脚本会随实现漂移）。

## 断点续跑提示
读本节 + 「需求背景」表格即恢复全部上下文；关键复用点：`TurnEventSink`（trait seam）+ `agent/sse_adapter.rs::format_sse_event`（帧格式**已有，勿重造**）。

## 工作量画像
- C1 JSONL/SSE 输出 + stdout/stderr 分离：0.5 人日
- C2 退出码分类学 + 失败传播：0.5 人日
- C3 结果封套：0.5 人日
- C4 审批策略（含 `ask_user` 收敛）：1 人日
- C5 有界执行（超时/进程树收敛）：0.5~1 人日
- C6 验收套件 + CI 接线 + 文档：1 人日
- **合计约 4~4.5 人日**（含门禁与对抗审查约 5 人日）

## OpenSpec Change
待建（立项时创建）。

## Canonical Spec
待建：建议 `openspec/specs/headless-delivery-form/spec.md`。

## Spec 状态
未起草。
