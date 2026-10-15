# Wave-13 冻结契约 — 修复 CI `verify` 作业红灯（PA-CI-TUI-LINT）

- 需求（用户原话）：**调用 teammate 解决这个红灯问题**（指 `main` 推送后 `CI` 工作流 `verify` 作业的失败步骤「Core 单元与集成测试（含 Windows Job 生命周期）」）。
- 分级：**B-lite**（单模块=Rust core 的 TUI bin + 其测试目标 + CI 门禁；无业务逻辑改动；Windows 专属失败，本地无法完整复现）。
- 基线 commit：`bd33ed5`（v0.1.145）
- 时序：Test Agent 先落盘「可复现的红相收据 + 前置断言脚本」→ Lead 锚定 → Executor 才可写 `src/**`/`tests/**`。

## 1. 根因（Lead 侦察，CI 日志 + 代码双向确认）

CI 失败步骤实际命令：`npm run cargo:test:shared -- --package=pony-agent-core`
→ `cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test --package=pony-agent-core`

CI 日志（`gh api .../actions/jobs/116384205446/logs`）末尾：

```
error: could not compile `pony-agent-core` (test "tui_acceptance") due to 17 previous errors
```

两个独立成因：

1. **测试构建的 dead_code 产物（15 条）**：`crates/pony-agent-core/tests/tui_acceptance.rs:1-2` 用
   `#[path = "../src/bin/tui.rs"] mod tui;` 把 **bin 目标当模块引入**，于是仅被 `main()` / `run_tui()` / `ui()` 触达的条目（`UiEvent`、`SelectingSession`、`clean_tool_name_display`、`DynamicProviderResolver`、`TuiTurnSink`、`build_control_plane`、`ui`、`run_tui`、`main` 等）在测试 crate 中判为 never used。
2. **Windows 专属真实警告（1 条）**：`crates/pony-agent-core/src/bin/tui.rs:1402` 的 `let log_file = log_dir.join("tui-runtime.log");` **只在 `#[cfg(unix)]` 分支内被使用** → 非 unix 目标下 `unused_variables`。

**为什么会被 deny**：`actions-rust-lang/setup-rust-toolchain@v1` 默认注入 `RUSTFLAGS=-D warnings`（工作流中「Rust 检查」步骤又显式声明同一值佐证），因此上述 warning 全部升级为 error。
**为什么只有 Windows 红**：`core-platform-check`（ubuntu/macos）只跑 `cargo check --package=pony-agent-core`，**从不编译 `tui_acceptance` 测试目标**；只有 `verify`（windows-latest）跑 `cargo test`。
**历史**：CI 自 2026-10-08 13:04（`5cd3333`）起持续失败，而 `42a9a1d`（10-08 10:04）仍 success；区间内提交均未触碰 TUI → 判定为**未固定的 Rust stable 工具链 + 该测试目标首次完整编译**共同作用，非单一提交引入。

## 2. 目标契约（冻结）

- **C1（本地红/绿门禁）**：`RUSTFLAGS="-D warnings" cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test --package=pony-agent-core` 在修复前 **EXIT≠0**（含 `test "tui_acceptance"` 编译错误），修复后 **EXIT=0**。
- **C2（Windows 专属警告消除）**：`src/bin/tui.rs` 中仅在 `#[cfg(unix)]` 内使用的绑定不得在非 unix 目标产生 `unused_variables`；修法必须**保持 unix 行为不变**（`dup2` 到 `STDERR_FILENO` 的日志重定向仍生效）。
- **C3（最小且可解释的 allow）**：`tests/tui_acceptance.rs` 的 dead_code 产物以**最小 lint 集合 + 中文注释说明「bin 当模块引入的固有产物，非真实死代码」**处理；**禁止** `#![allow(warnings)]` 之类全局放水，禁止删除或 `#[ignore]` 既有测试用例。
- **C4（不得绕道）**：不得删除 `tui_acceptance` 测试目标、不得修改 `.github/workflows/ci.yml` 的命令或移除 `-D warnings`。
- **C5（回归门禁）**：修复后本地 `RUSTFLAGS="-D warnings" cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check --package=pony-agent-core` EXIT=0；`npm run test:unit` / `npm run typecheck` 保持 EXIT=0。
- **C6（Windows 物理凭证）**：以 GitHub Actions 为准 —— 推送分支并开 PR（触发 `pull_request`），`CI / verify` 作业必须 **success**（唯一可得的 Windows 真机证据；本地无 Windows 工具链/交叉 std）。

## 3. 交付物

| 角色 | 交付物 | write_scopes |
| :-- | :-- | :-- |
| Test Agent | `scripts/check-rust-test-lints.mjs`（跨平台前置断言脚本：执行 C1 精确命令并按 EXIT 判定）+ 红相收据 `.dev-team/evidence/red-phase-wave13.log`（含 17 条错误清单与 EXIT）+ 绿相复跑收据 | `scripts/`、`.dev-team/evidence/` |
| Executor | `crates/pony-agent-core/src/bin/tui.rs`（C2）、`crates/pony-agent-core/tests/tui_acceptance.rs`（C3） | 上述两文件 |
| Lead | 契约、红相锚定、PR 创建与 CI 观察、最终验收、ADR（若需） | `.dev-team/`、git 操作 |

## 4. 非目标（Non-Goals）

- 不重构 TUI 结构（不把 `tui.rs` 状态/类型搬进 lib）；
- 不引入 `rust-toolchain.toml` 固定工具链（另开单评估）；
- 不修 CI 中其它历史问题（如 `Node.js 20 deprecated` 警告）。

## 5. 侦察补全（Lead，2026-10-15）：**完整失败目标矩阵**

Test Agent 独立发现 `cli_acceptance` 同样失败（C1 聚合命令在本机首个失败目标随 cargo 调度顺序变化，CI 日志只呈现其中一个）。Lead 遂对 `crates/pony-agent-core` 的**全部测试目标逐一编译**（`RUSTFLAGS="-D warnings" cargo test --test <name> --no-run`，30 个集成测试目标 + `--lib` + `--bins`），原始矩阵存档 `.dev-team/evidence/lead-recon-target-matrix.txt`：

| 目标 | EXIT | 错误数 | 性质 |
| :-- | --: | --: | :-- |
| `tui_acceptance` | 101 | 16 | 15-16 条 bin-as-module `dead_code`（`main`/`run_tui`/`ui`/`build_control_plane`/`TuiTurnSink`/`DynamicProviderResolver`/`ascii_logo`/`add_tool_event`/`open_session_selector` 等） |
| `cli_acceptance` | 101 | 13 | 1 条**真实** `unused import: CliArgs` + 12 条 bin-as-module `dead_code`（`TerminalSink`/`SessionTracker`/`handle_init`/`print_help`/`main` 等） |
| `trace_auto_report_test` | 101 | 2 | **真实** `unused import: AgentTracePayload`、`unused import: serde_json::json` |
| 其余 27 个集成目标 + `--lib` + `--bins` | 0 | 0 | 通过 |

**Windows 专属（本机不可见）**：`src/bin/tui.rs:1402` 的 `log_file` 仅在 `#[cfg(unix)]` 块（`:1403-1408`）内使用 → 非 unix 目标 `unused_variables`。它既影响 `tui_acceptance` 的模块引入，也会在任何**非缓存**的 Windows `cargo check --bins` 上触发，故必须真修（把声明移入 `#[cfg(unix)]` 块），不得仅靠测试侧 allow 掩盖。

## 6. 修正后的写域（Executor）

| 文件 | 处理 |
| :-- | :-- |
| `crates/pony-agent-core/src/bin/tui.rs` | C2：`log_file` 声明移入 `#[cfg(unix)]` 块（unix 行为不变） |
| `crates/pony-agent-core/tests/tui_acceptance.rs` | C3：`mod tui;` 上加最小 lint allow + 注释 |
| `crates/pony-agent-core/tests/cli_acceptance.rs` | 移除真实 `unused import: CliArgs`；对 bin-as-module `dead_code` 加同款最小 allow + 注释 |
| `crates/pony-agent-core/tests/trace_auto_report_test.rs` | 移除两处真实 unused import（**不得**用 allow 掩盖） |

**C1 验收命令升级为全目标**：`RUSTFLAGS="-D warnings" cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test --package=pony-agent-core --no-run` 必须 EXIT=0（等价于 CI 步骤的编译面）；并逐一复核上表 3 个曾失败目标 EXIT=0。
