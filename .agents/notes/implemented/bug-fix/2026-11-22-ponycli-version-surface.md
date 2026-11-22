# Agent Note: `ponycli --version` 进程级版本面（B261 前半）

Status: implemented

## Problem

`pony`（TUI）支持 `--version` / `-V`（`src/bin/tui.rs` 的 `CliAction::Version`），但 `ponycli`
没有对应分支：`--version` 会落到 `parse_cli_args` 的非法参数分支，表现为 `Argument Error` + Usage(2)，
**没有任何版本输出**。后果：

1. 脚本/安装器无法探测 CLI 版本（`ponycli --version` 失败被当成「命令不存在」）；
2. `pony` 与 `ponycli` 同包发布，自更新后两者的实际版本对用户不可见，漂移只能靠猜；
3. 现有门禁（`npm run version:check`）只保证**仓库内四处文件**一致，无法证明**装到机器上的二进制**
   真的报出这个版本。

## Decision

1. **在 `main` 最前面拦截** `--version` / `-V`（先于 observability 装配、参数解析与配置读取），
   输出单行 `ponycli <version>` 并退出 0。与既有的「维护子命令」「ACP 子命令」同属
   「解析之前拦截」的命名空间，避免再被解析器吞掉。
2. **版本真源不写第二份**：直接用 `env!("CARGO_PKG_VERSION")`（workspace 同一 `Cargo.toml`，
   与 `pony` 天然同源）。四处版本同步仍由 `scripts/check-version-sync.ps1` 门禁负责，
   本改动不引入任何版本字面量。
3. **用进程级测试锁定行为**（`tests/ponycli_version_test.rs`，通过 `CARGO_BIN_EXE_ponycli` 真跑二进制）：
   断言 stdout 恰为 `ponycli <CARGO_PKG_VERSION>` 且退出码 0；`-V` 等价。
   只测内部函数无法证明「没被参数解析吞掉」——而 B261 的症状正是这一点。
4. **范围切分**：B261 的另一半「随 pony 自更新后静默版本漂移（同包两 bin 无一致性提示）」
   属于安装/更新路径（`scripts/install-cli.mjs` 的原子替换与自更新流程），与「版本输出面」不是同一主题，
   拆到后续 PR；B261 保持 `todo` 并在账本注明拆分。

## Alternatives considered

- **引入 `clap` 接管 `ponycli` 参数**：与现有手写解析器并存会形成两套解析语义（`--output-format` 等
  已有严格校验与 fail-closed 默认值），迁移成本与回归面远超一条版本输出；TUI 侧也是手写解析，保持一致。
- **只在 `parse_cli_args` 里加 `--version` 分支**：能工作，但它把版本输出绑在「参数解析成功之后」，
  仍要装配 observability、仍可能被后续新增的前置校验截断；放在 `main` 最前面语义最直白。
- **只加单元测试测 `version_banner()`**：同义反复，证明不了「`ponycli --version` 能退出 0 并打印」，
  正是 B261 的失败形态。
- **版本号写死成常量**：与「版本四处同步」门禁形成第二真源，必然漂移。
- **`--version` 里调用 `pony --version` 做一致性提示**：多一次进程启动（桌面环境可达数百 ms），
  且在两 bin 版本不一致时到底报谁仍是未决问题；留给安装器侧解决更合适。
