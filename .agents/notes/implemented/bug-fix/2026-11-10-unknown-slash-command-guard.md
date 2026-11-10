# Agent Note: TUI 拼错的斜杠命令在分发前拦截（ADR 0114 实施记录）

Status: implemented

- 日期：2026-11-10
- 决策真源：[`docs/decisions/0114-tui-unknown-slash-command-guard.md`](../../../docs/decisions/0114-tui-unknown-slash-command-guard.md)
- 交付面：`pony`（TUI 二进制）

## Problem

TUI 输入框敲错斜杠命令（`/cleer`、`/thikning`、`/clearx`）时，`parse_command_input` 返回 `None`，
调用方据此把整串当**普通消息**发给模型——命令语义丢失、用户没有任何提醒，还白烧一次 LLM 调用
（流式期先排队，下一轮再作为聊天正文发出）。用户诉求：不在当前命令表内的斜杠命令**禁止发出**，
且有提醒。

## Decision

1. **判定面（纯函数）**：`tui/projection.rs` 新增 `unknown_slash_command(input) -> Option<&str>`
   ——取 `trim()` 后首 token，要求「单个前导 `/` + 仅含字母/数字/`-`/`_`」（命令词形），
   且不命中 `AVAILABLE_COMMANDS`、不命中 `HISTORIC_COMMAND_ALIASES`（`/exit`、`/effort`）。
   提示文案唯一真值：`unknown_command_notice(token)`。
2. **拦截点一 · 非流式主分发**（`bin/tui.rs`，`parse_command_input` 之后、命令分支之前）：
   `state.set_input(&input)` 保留原文 → `apply_queue_feedback(SystemMessage)` 落消息区 + 状态行 →
   结构化日志 `composer_command_rejected{reason=unknown_slash_command}` → `continue`（不派发轮次）。
3. **拦截点二 · 流式期入队前**（`StreamingKeyDisposition::QueueSubmit` 分支，`push_queued_message`
   之前）：同文案同通道拦截，输入保留在输入框；先入队再拦会让正文被摘出输入框却永远发不出去。
4. **零语义外溢**：`parse_command_input` / `match_command_token` / `push_queued_message` 一字不改；
   路径形输入（`/etc/hosts 是什么`）、已知命令 + 正文（`/clear 这个需求怎么拆`）继续按普通消息发送。

## Alternatives considered

详见 ADR 0114「候选方案」：B 凡 `/` 开头一律拦（与契约 F-B 冲突）、C 改 `parse_command_input`
自带哨兵（三处共用口径 + N2 矩阵冻结）、D 拦下后清空输入框（逼用户重敲）、E 只在桌面端拦（缺陷在 TUI）。

## Consequences

- 机器回执：`cargo test -p pony-agent-core --bin pony`：111 passed（含新增
  `unknown_slash_command_tests` 9 例）；`--test tui_batch1_regression`：120 passed（N2 口径不变）；
  `--test tui_streaming_input_acceptance` / `--test tui_streaming_input_adversarial` /
  `--test slash_commands_acceptance` / `--test w3_team_visibility_test` / `--test tui_six_fixes_acceptance`
  全部通过。
- 新用例文件：`crates/pony-agent-core/src/bin/tui/tests/unknown_slash_command_tests.rs`（9 例，
  正例：拼错大小写/trim/带正文；反例：命令表全命中、别名、路径形、裸 `/`、纯文本）。
- 残余风险（靠 review）：判定面是字符串形状启发式——`/中文` 这类字母数字混排 token 会被判为命令词
  而拦下；将来若有「以 `/` 开头的非命令自由文本」诉求，扩命令表或改判定面，而不是放宽边界。
