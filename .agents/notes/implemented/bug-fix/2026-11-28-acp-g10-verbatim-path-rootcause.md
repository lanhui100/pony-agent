# Agent Note: g10 Windows 不落盘的根因是 verbatim 路径形态，不是并发模型

Status: implemented

## Problem

`acp_capabilities_contract::g10_slow_inbound_request_does_not_starve_others`
（`crates/pony-agent-core/tests/acp_suite/acp_capabilities_contract.rs`）在 Windows runner 上
**应答文件完全缺失**：用例卡在 `captured_responses` 的
`fake agent 必须捕获入站应答（…responses.jsonl：No such file or directory）`，而同批其余 10 例通过。
当时（B1795482668208）只能整例 `#[cfg_attr(windows, ignore)]`，注释写的是「是 `sh -c` 子进程
span 失败还是 argv 模式匹配退化尚无证据」。

## Decision

不再猜并发/argv，回到**该用例与本文件其它 probe_* 用例的唯一差异**：

1. 同文件的 `spec_capturing_initialize` / `spec_probing_fs` / `spec_probing_terminal` 在把
   `<initFile>` / `<respFile>` 交给 fake agent 之前，全部先过 [`shell_path`]；
2. 只有 g10 直接用 `PathBuf::to_string_lossy()` 拼参数；
3. `temp_dir()` 走的是 `canonicalize()`，Windows 上产出 `\\?\C:\...` 的 **verbatim** 前缀——
   而 POSIX shell（Git Bash）按该形态 `>> file` 会静默失败。这正是同文件 `shell_path`
   文档里记的 B230 结论（「capture 落盘静默失败」）。

于是修复 = **让 g10 也走 `shell_path`**，并撤除 `#[cfg_attr(windows, ignore)]`。
同时把 h4（`acp_config_regressions::h4_long_running_terminal_wait_respects_budget_and_stays_responsive`）
的头注释收窄：它的夹具 `probe_terminal_long` 是严格同步的（9404 只在 9402 应答之后才发），
**结构上观测不到 wait 期间的并发**。「慢请求不饿死其它请求」这条不变量由 g10 的
`probe_terminal_interleave`（wait 在途时立刻发 9404、不等应答）承担；h4 只保留它真能测的两条：
预算不被 5s 硬顶、wait 后同轮请求仍被应答（存活断言，不是并发断言）。

## Alternatives considered

- **继续 `ignore` 等 Windows 侧抓 stderr**：ignore 会让这条不变量在 Windows 上**永久无覆盖**，
  而以「同文件其它用例都过、只有这一例没走 `shell_path`」为线索的成本远低于重新造夹具。
- **把夹具改成纯 Rust 可控时序（不依赖 `sh` 与 `sleep`）**：能彻底摆脱 POSIX shell，
  但要重造 ACP 子进程夹具（JSON-RPC 读写循环 + 16 个模式），与本缺陷的因果关系无关；
  且 `sleep 3` 本身不是失败点（同批其它用例也用 `sh -c` 子进程）。
- **在 Windows 上改用 `cmd.exe` 跑夹具**：等于给同一份 `.sh` 夹具加两套启动语义，
  维护面翻倍，且会让「同一夹具在 CI 与本地跑不同东西」。
- **把 h4 的「不饿死」断言删掉而不改注释**：删断言会让 h4 看起来像从未测过并发；
  真正的处理是把口径写清楚、把责任指到承担它的用例上（g10）。

## Consequences

- g10 在 Windows 上重新生效（无 `ignore`）；Linux 侧行为与之前一致。
- 教训记在案：**跨平台 shell 传参必须过 `shell_path`**，任何新 probe_* 用例都按此口径写。
- h4 的断言范围与它的名字一致了（「respects budget and stays responsive」而非「does not starve」）。

## Verification

- `bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_suite g10_slow_inbound` → 1 passed
- `bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_suite` → 144 passed / 0 failed / 0 ignored
- Windows 侧的最终确认由 CI 的 Windows job 承担（本机为 Linux，无法本地复现 verbatim 前缀）。
