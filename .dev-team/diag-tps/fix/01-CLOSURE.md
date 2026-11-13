# 收口：遗留事项按建议顺序逐项实施（2026-11-13）

- 用户指令：「按照建议顺序逐项实施」。顺序 = 0 保护性快照 → 1 ADR/backlog → 2 定向提交 → 3 可观测性声明 → 4 低优先 backlog。
- 全部动作**只新增/修改本单文件**；另一并行会话的 WIP 原样保留，未 `stash`/`checkout`/`reset`。

## 0 保护性快照 ✅
- `.dev-team/diag-tps/fix/patches/f1f2f3-20261113-081217.diff`（2907 行，18 个受控文件相对 HEAD 的差异）
- `.dev-team/diag-tps/fix/patches/f1f2f3-untracked-20261113-081217.tgz`（4 个新验收测试）
- 两份均随 `eb1a0098` 一起入库，可随时 `git apply` 恢复。

## 1 ADR + backlog ✅
- 新增 [ADR](../../../../.agents/notes/implemented/bug-fix/2026-11-13-token-speed-evidence-and-window-purity.md)：
  `implemented/bug-fix`，含 `## Problem` / `## Decision`(D1–D4) / `## Alternatives considered`(A–E) / `## Consequences`，
  链入同族 `2026-11-12-metrics-percent-honesty.md` 与速度口径契约 v2。
- `npm run adr:check` → **Exit 0**（1.1–1.10 + 117 条归档封印一致）。
- backlog：`B124`（turn/speed 等价性用例）与 `B134`（本单）已 `resolve` 并归档；
  `B135`（span 残差）/ `B136`（慢 sink 解耦）/ `B137`（4 条 flaky）/ `B139`（上游 429 需记 HTTP 码）登记为 todo。
- `bash .meta/gates/backlog-gate.sh` → **Exit 0**（活跃 17 / 归档 65）。

## 2 提交 ✅
- 用户裁决：合并提交本单 + 另一会话已暂存的 8 个文件（共用 `retry.rs` / `provider/mod.rs`，物理不可无损拆分）。
- 提交前收据：`git diff --cached --name-only` 逐项核对；`secrets:check --staged` 0 leaks；排除 `temp-eval-env/` 与临时脚本。
- `eb1a0098 fix(metrics): t/s 口径可取证 + 重试窗口纯净 + 读循环去阻塞 [B134,B124]`（347 文件，含并行 WIP，消息已逐项点名）。
- **提交树独立验证**：`git archive HEAD` 解到 `/tmp/pa-head-tree`，`cargo check -p pony-agent-core` → **Exit 0**。
- 期间另一会话在写 `system_prompt_template` 模块拆分，工作区曾短暂出现 `E0432`；确认 HEAD 内 0 处引用、`cargo check` 工作区最终 **Exit 0**，属并行写入的中间态，非提交缺陷。

## 3 可观测性声明 ✅
- `docs/observability.yaml` 追加本单归属登记（四维归属不变、metrics/logs 结构性 N-A、不新增 enforcement_point、收口点=4 个测试文件 + 静态门）。
- `426fa1d6 docs(observability): 登记 t/s 口径修复的本地取证面 [decl-change-approved] [B134]`。
- `audit-diff.sh --base 23091fa3` → **Exit 0**（"声明变更已显式批准" + 无越界实现、收口点完整）。
- `check-declaration.sh /home/dm/pony-agent` → **Exit 0**（tracing/issues/logs/metrics 四维 PASS）。
- 附带效果：此前全量 vitest 的 `wave47-ob2 T7` 与 `pds-streaming V10-3` 两条红因声明获批而消失。

## 4 低优先 backlog ✅
| ID | 事项 |
|---|---|
| B135 | `llm.chat` span per-call `dur` 仍含重试等待（需 peek / 传纯净窗口） |
| B136 | 慢 sink 串读循环：发送线程/队列解耦 |
| B137 | `--lib` 4 条负载敏感 flaky 用例治理 |
| B139 | 上游测速记录 HTTP 码（`gemini-3.8-flash-high` 8/12 次 429） |

## 终局门禁收据（全量）
| 命令 | 结果 |
|---|---|
| `npm run version:check` | Exit 0（core 0.1.163 / tauri 0.1.190 四处同步） |
| `npm run adr:check` | Exit 0 |
| `npm run secrets:check --staged` | Exit 0（0 leaks） |
| `bash .meta/gates/backlog-gate.sh` | Exit 0 |
| `npm run test:unit`（vitest 全量） | **164 files passed / 1530 passed / 12 skipped / 0 failed** |
| `npm run build`（vue-tsc + vite build） | Exit 0 |
| `npm run contract:check` / `contract:cli` | Exit 0（Rust/TS 键一致） |
| `npm run size:check` | Exit 0（**0 new violation**，此前 2 条已通过行中性化消除） |
| `cargo check -p pony-agent-core --all-targets` | Exit 0（7m25s） |
| `npm run ext:check` | Exit 0 |
| `audit-diff.sh --base 23091fa3` / `check-declaration.sh` | Exit 0 / Exit 0 |
| `cargo test -p pony-agent-core --lib` | Exit 0（1527 passed / 0 failed） |
| 三套 F1/F2/F3 验收 + 变异红相 | 见 `red-receipt.md` |

**唯一环境限制**：`npm run verify` 整条链在 `cargo:check:shared` 处报 `sh: powershell: not found`（exit 127）——
该脚本硬编码 Windows PowerShell，本机 Linux 无 `pwsh`；上面逐项手工重跑其全部前后步骤均 Exit 0。
这不是本单引入的缺陷，也不改变门禁结论，但完整 `npm run verify` 在本机结构上不可绿，需另立环境单或改跨平台包装。

## 工作区收尾
- 仍为脏的是**另一并行会话**的 9 项文件（`context.rs` / `mod.rs` / `prompt_contract.rs` 等的 system-prompt 拆分，
  以及 `temp-eval-env/`、临时脚本）；本单不再触碰。
- 本单所有产物（诊断、三修复、ADR、声明、收据、patch）均已入库到 `eb1a0098` + `426fa1d6`。
