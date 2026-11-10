# ADR 0114：TUI 拼错的斜杠命令在分发前拦截（契约 F-D）

Status: implemented

日期：2026-11-10
实施说明：[`.agents/notes/implemented/bug-fix/2026-11-10-unknown-slash-command-guard.md`](../../.agents/notes/implemented/bug-fix/2026-11-10-unknown-slash-command-guard.md)。
关联：契约 F-B（`crates/pony-agent-core/src/bin/tui/projection.rs::parse_command_input`，已知命令 + 正文必须继续当普通消息发送）口径不变。

## 背景

TUI 输入框支持斜杠命令表（`AVAILABLE_COMMANDS`，16 条）。用户敲错命令（`/cleer`、`/thikning`、
`/cost`）时的现状是**静默发出去**：`parse_command_input` 对未命中返回 `None`，调用方据此把整串
当普通消息发给模型。后果有两个：

1. 用户以为命令执行了，其实模型把 `/cleer` 当正文回答（命令语义丢失，提示缺失）；
2. 一次无意义的 LLM 调用（流式期还会先排队，再在下一轮变成聊天消息）。

需求口径：**不在当前命令表内的斜杠命令禁止发出，并给出提醒。**

## 候选方案

- **A（采纳）首 token「命令词形」判定 + 分发前拦截**：新增纯函数
  `unknown_slash_command(input)`——取 `trim()` 后首 token，要求「单个前导 `/` + 仅含字母/数字/
  `-`/`_`」（即真的长得像命令词），且不命中命令表、不命中历史别名（`/exit`、`/effort`）才算拼错。
  在两个入口拦截：非流式主分发、流式期入队前。拦下时保留原文、走 SystemMessage + 状态行提示。
- **B（落选）凡 `/` 开头一律拦截**：把 `/etc/hosts 是什么`、`/usr/bin/env` 这类以路径开头的正常
  提问也拦住，与契约 F-B「不吞用户正文」正面冲突（同样的理由当年否决了「首 token 命中即当命令」）。
- **C（落选）改 `parse_command_input` 自带哨兵**：该函数是**三处共用口径**
  （主分发、`queue::push_queued_message`、`queue::streaming_enter_pre_action`），改语义会同时
  改变菜单补全/入队判定，且 `tests/tui_batch1_regression.rs` 已冻结其 N2 矩阵。新增独立判定面
  比扩宽既有真值源的语义更小、更可断言。
- **D（落选）拦下后清空输入框**：拼错需要就地改一个字，清空等于让用户重敲整行（与
  「正文不静默丢弃」的既有取向相反）。
- **E（落选）只在桌面端拦**：缺陷报告来自 TUI 输入框，桌面端走另一条输入通路，与本卡无关。

## 决策

1. **判定真值源**：`crates/pony-agent-core/src/bin/tui/projection.rs` 新增
   `unknown_slash_command()` 与历史别名表 `HISTORIC_COMMAND_ALIASES`；提示文案唯一真值
   `unknown_command_notice()`。
2. **拦截点一（非流式主分发）**：`bin/tui.rs` 在 `parse_command_input` 之后、任何命令分支之前拦下，
   把原文写回输入框（`set_input`），经 `apply_queue_feedback(SystemMessage)` 落消息区 + 状态行，
   并写一行结构化日志 `composer_command_rejected{reason=unknown_slash_command}`。
3. **拦截点二（流式期）**：`StreamingKeyDisposition::QueueSubmit` 分支在 `push_queued_message`
   **之前**拦截——先入队再拦会让正文被摘出输入框却永远发不出去（静默丢正文）。
4. **不动的口径**：`parse_command_input`、`match_command_token`、`push_queued_message` 语义一字不改；
   路径形输入（含第二个 `/`）、已知命令 + 正文（`/clear 这个需求怎么拆`）继续按普通消息发送。

## 后果

- 拼错的命令不再产生 LLM 调用；用户拿到「未知斜杠命令「/cleer」· 已拦截未发送 · 输入 /help …」，
  且输入框保留原文可直接改字重发。
- 行为可机判：`crates/pony-agent-core/src/bin/tui/tests/unknown_slash_command_tests.rs`（9 例，
  正例/反例双面：命令表全命中放行、别名放行、路径形放行、带正文放行）。
- 残余风险（靠 review）：判定面是**纯字符串形状启发式**。`/中文` 这类字母数字混排 token 会被判为
  命令词而拦下；若将来出现「以 `/` 开头的非命令自由文本」诉求，应扩命令表或改判定面，而不是放宽
  「形如命令词」这条边界。
