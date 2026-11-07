# CLI 入口与版本/子命令面侦察结论

## 1. 两个二进制的定位与交互程序归属
- 结论 — 两个二进制分别为全屏 TUI 交互程序（`pony`）与命令行/无头/REPL 交互程序（`ponycli`）；`ponycli` 才是「命令行 Agent 交互程序」 — 证据: `crates/pony-agent-core/Cargo.toml:9-15`，`crates/pony-agent-core/src/bin/cli.rs:1`，`crates/pony-agent-core/src/bin/tui.rs:1`

## 2. `ponycli` 的子命令与参数解析
- 结论 — `enum SubCommand` 变体仅 `Run`, `Init`；`enum SlashCommand` 变体为 `Clear`, `Compact`, `Help`, `Exit`, `Unknown(String)` — 证据: `crates/pony-agent-core/src/bin/cli.rs:66-78`
- 结论 — `parse_cli_args` 结构为先匹配子命令 `init`，再 `while` 循环匹配参数 flag 与 prompt — 证据: `crates/pony-agent-core/src/bin/cli.rs:179-298`
```rust
pub fn parse_cli_args(args: &[String]) -> Result<CliArgs, String> {
    let mut config = CliArgs::default();
    let mut idx = 1;

    if idx < args.len() && args[idx] == "init" {
        config.subcommand = SubCommand::Init;
        idx += 1;
    }

    while idx < args.len() {
        match args[idx].as_str() {
            "-p" | "--prompt" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--prompt requires a value".to_string());
                }
                config.prompt = Some(args[idx].clone());
            }
            "-m" | "--model" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--model requires a value".to_string());
                }
                config.model = args[idx].clone();
            }
```
- 结论 — 新增 `update` 子命令/参数应在 `enum SubCommand`（cli.rs:66-69）新增变体，并在 `parse_cli_args` 初始子命令判断处（cli.rs:183-186）或参数循环匹配分支（cli.rs:189）插入解析，在 `main()` 派发处（cli.rs:1131-1137 之后）插入执行逻辑 — 证据: `crates/pony-agent-core/src/bin/cli.rs:66-69`，`crates/pony-agent-core/src/bin/cli.rs:183-186`，`crates/pony-agent-core/src/bin/cli.rs:1131-1137`

## 3. `pony` (tui.rs) 的参数解析入口与子命令现状
- 结论 — `pony` 的参数解析入口为 `parse_cli_action`（tui.rs:13028-13045）；已解析 `--version`/`-V`、`--help`/`-h`、`--doctor`、`--demo-ask-user` 以及子命令 `resume [<session-id>]`；加 `update` 的插入点在 `CliAction` 枚举定义（:13017-13026）、`parse_cli_action` 匹配分支（:13038 之前或之后）以及 `main()` 派发分支（:13308-13358） — 证据: `crates/pony-agent-core/src/bin/tui.rs:13017-13045`，`crates/pony-agent-core/src/bin/tui.rs:13308-13320`

## 4. `--version` 输出来源与拼装位置
- 结论 — `--version` 来自宏 `env!("CARGO_PKG_VERSION")`；`pony` 在 `tui.rs:13086-13097`（`cli_version_string`）拼装（带 OS、架构与二进制路径）；`ponycli` 目前**未实现** `--version` 选项，仅在 help 说明中引用了版本概念，Cargo 层面版本定义在 `Cargo.toml:3` — 证据: `crates/pony-agent-core/src/bin/tui.rs:13086-13097`，`crates/pony-agent-core/src/bin/cli.rs:978-1016`，`crates/pony-agent-core/Cargo.toml:3`

## 5. 进程退出码约定
- 结论 — `ponycli` 退出码统一定义于 `ExitCode` 枚举（`Success=0`, `Internal=1`, `Usage=2`, `Config=3`, `ProviderNetwork=4`, `ToolFailure=5`, `PermissionDenied=6`, `Timeout=7`），通过 `finish(code, observability)` 终结进程；新增 `update` 失败时若是参数错误遵循 `ExitCode::Usage`(2)，网络/API 请求失败遵循 `ExitCode::ProviderNetwork`(4) 或 `ExitCode::Internal`(1) — 证据: `crates/pony-agent-core/src/bin/cli.rs:1101-1104`，`crates/pony-agent-core/src/bin/cli.rs:1122`，`crates/pony-agent-core/src/agent/headless.rs:96-121`

## 6. `update` / `upgrade` / `self_update` 字样或占位
- 结论 — `cli.rs` 中**完全不存在**任何 `update` / `upgrade` / `self_update` 字样；而 `tui.rs` 中已存在完整的 `/update` 斜杠命令实现（定义了 `UpdateCheckOutcome`、`perform_update_check()` 检查 GitHub Releases 与 S3、`format_update_feedback()` 格式化文案） — 证据: `crates/pony-agent-core/src/bin/cli.rs:1-1280`（grep 0 命中），`crates/pony-agent-core/src/bin/tui.rs:2020-2139`

## 7. 输出/日志惯例与结构化事件支持
- 结论 — `cli.rs` 中人类可读输出使用宏 `human!` / `human_inline!`（`Text` 模式输出 stdout，`jsonl`/`sse` 模式自动改道 stderr），报错使用 `eprintln!`；已支持 `--output-format (text|jsonl|sse)`，若 `update` 需要输出结果，若按子命令执行，标准人类文案应通过 `human!` 或 `eprintln!` 输出，若纳入结构化事件则需在 `jsonl`/`sse` 下通过 `TurnEventSink` 输出特定事件或遵循 `ResultEnvelope` 格式 — 证据: `crates/pony-agent-core/src/bin/cli.rs:46-63`，`crates/pony-agent-core/src/bin/cli.rs:237-245`，`crates/pony-agent-core/src/bin/cli.rs:1145-1151`

---

## 覆盖盲区
- 完整阅读行范围：
  - `crates/pony-agent-core/Cargo.toml`: 1-100（覆盖 package, bin, dependencies 定义）
  - `crates/pony-agent-core/src/bin/cli.rs`: 1-380, 700-844, 950-1280（覆盖宏、枚举定义、参数解析、单发/REPL流程、退出码映射与 main 流程）
  - `crates/pony-agent-core/src/bin/tui.rs`: 1-120, 2020-2139, 13010-13420（覆盖头部说明、update 检查实现、CLI 参数动作解析、main 函数派发）
  - `crates/pony-agent-core/src/agent/headless.rs`: 90-170（覆盖 ExitCode 变体与数值）
- 未阅读行范围：
  - `crates/pony-agent-core/src/bin/cli.rs`: 381-699, 845-949（此部分为 Agent 控制面构建、流式 Sink 回调与 stdin 处理等中间执行辅助函数，不影响 CLI 参数面与退出码契约判定）
  - `crates/pony-agent-core/src/bin/tui.rs`: 121-2019, 2140-13009, 13421-14207（此部分为 ratatui UI 布局渲染、按键处理与单元测试等纯 TUI 视图代码）
  - `crates/pony-agent-core/Cargo.toml`: 101-159（仅为剩余 targets 和 features 配置）
