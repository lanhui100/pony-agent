# Agent Note: herdr 复用器变量名漂移（Enter 变 `u`）与会话索引行丢失（`pony resume` 谎称无历史）

Status: implemented

## Problem

一条真实用户现场，两个独立缺陷在同一条链路上爆出：`pony` 退出时提示 `pony resume pa_01a1b69c-…`，
把该命令粘贴回新 shell 后，**① 按 Enter 不提交、行尾多出一个 `u`**；**② TUI 说「没有该会话 / 新会话」**。

- ① 现场终端是 herdr 0.7.5。实测 `/proc/<pid>/environ`：herdr 只注入 `HERDR_ENV=1`、
  `HERDR_PANE_ID`、`HERDR_SOCKET_PATH`、`HERDR_TAB_ID`、`HERDR_WORKSPACE_ID`，**没有**裸 `HERDR`；
  而 ADR 0075 的复用器清单只认 `HERDR`/`TMUX`/`ZELLIJ`/`STY`/`ABDUCO`。于是 herdr 被判成直连终端
  ⇒ push `DISAMBIGUATE|REPORT_ALL_KEYS`(`CSI >9u`) ⇒ herdr 不解析 kitty 协议、pop 不透传，
  退出后 Enter 仍被编码成 `CSI 13u`。PTY 复现（bash）：发 `CSI 13u` → readline 输出 `\x07u`，
  与用户描述的「回车不生效 + 多一个 u」逐字符一致。
- ② 真实库 `~/.local/share/PonyAgent/sessions.db`：`pa_01a1b69c-…` 的 `normalized_sessions`(1)、
  `normalized_messages`(6)、`normalized_turn_traces`(3)、`normalized_history_nodes`、`turn_events`(1266)
  全在，唯独 legacy `sessions` 那行 blob 不见了；而 `SessionStore::list_sessions()` 只读 legacy 行
  ⇒ `existed=false`。能删这一行的只有 `write_full_store` 的兜底：整包写盘时把「内存快照里没有的
  会话」逐行 DELETE。多开实例（现场 4 个 `pony` + herdr 多 pane）时，任何一份**启动时装载的旧快照**
  一旦整包写盘（工作区引导回写 / 元数据回写 / 历史图修复），就会删掉别的实例新建的会话。
  旁证：三分钟内 `sessions` 表 72→71 行且最新行换 id；全库 73 条 legacy vs 121 条规范化行。

## Decision

1. `tui::TERMINAL_MULTIPLEXER_ENV_KEYS` 按**真实现场变量名**扩写为
   `HERDR`/`HERDR_ENV`/`HERDR_PANE_ID`/`HERDR_SOCKET_PATH`/`HERDR_TAB_ID`/`HERDR_WORKSPACE_ID`/
   `TMUX`/`ZELLIJ`/`STY`/`ABDUCO`，类型 `[&str; 5]` → `&[&str]`（清单会继续长）。
   herdr 下只 push `DISAMBIGUATE`：Shift+Enter 照常可用，pop 丢失时 Enter 仍是 `\r`。
   三处同口径同步：`tools/tui-audit/check_shift_enter.py` 的 `MULTIPLEXER_ENV_KEYS`、
   该脚本 `a2.5` 的注入环境（改为 herdr 真实形状 `HERDR_ENV=1`，旧断言只注 `HERDR`、
   清单漏写也照样变绿）、`tests/tui_startup_input_guard.rs` 内嵌 harness 清单。
2. `write_full_store` 去掉「对齐内存快照」的 DELETE（含 `session_turn_traces`）。整包写盘的语义是
   「把我知道的写下去」，不是「把库对齐成我知道的样子」。删除会话的唯一路径是显式 `remove_session`，
   它现在在同一事务里删 legacy 行 + trace + **规范化行**。
3. resume 按需自愈：新增 `SessionBackend::load_normalized_session(id)`（默认 `None`）、
   `SessionStore::recover_session_from_normalized(id)`、`HostControlPlane::recover_session_from_normalized`。
   `pony resume <id>` 在 `existed=false` 时先按 id 从规范化表重建、回写 legacy 行，再走既有 hydrate，
   并在状态行与首条 System 消息里如实告知「索引行曾丢失，已重建」。**只在显式 resume 触发**，
   不做启动期全量合并（否则会复活用户显式删除后仅剩规范化行的残留）。

## Alternatives considered

- **A：把 `HERDR_ENV` 作为特例硬编码进 `terminal_multiplexer()`**。落选：判定点分裂成「清单 + 特例」，
  下个复用器改变量名时又要改两处；清单本身就是分级决策的输入。
- **B：herdr 下完全不 push 键盘增强**。落选：Shift+Enter 退化成裸 Enter（＝直接发送），
  用删功能换稳定性，与 ADR 0075 的取舍一致地不可接受。
- **C：退出时无条件追加 `CSI = 0 u` 置零**。落选：置零已在还原串里，本缺陷根因是 push 面过大
  （`REPORT_ALL_KEYS`），置零只能事后补救，不能阻止退出前的乱码。
- **D：整包写盘改成「合并写」（先读库内全集再 upsert）**。落选：只是把删除换成「读全库 + 写全库」的
  更大事务，跨实例的丢更新仍在；写盘语义应止步于「把我知道的写下去」。
- **E：启动时把规范化表里所有会话合并回内存**。落选：会把用户显式删除后的残留自动复活，
  与「删除要留得住」冲突；故收敛为显式 resume 才自愈。
- **F：`pony resume <id>` 找不到时直接报错退出**。落选：退出提示里的 id 常来自尚未落库的会话，
  严格校验会把提示变成假承诺。

## Consequences

- 用户可见：herdr 下退出 TUI 后 Enter 恢复正常；被误删索引行的会话可用提示里的命令恢复出真实历史；
  多开实例不再互相抹掉会话。**历史已损坏的 48 个会话仍不在启动列表里**（只能按 id 显式 resume 自愈），
  双写一致性债由 [ADR 0093](../../../docs/decisions/proposed/0093-session-storage-event-sourced-refactor.md)（proposed）承接。
- 契约面：`SessionBackend` 新增带默认实现的方法（非破坏性）；`TERMINAL_MULTIPLEXER_ENV_KEYS` 类型变更，
  唯一消费者是 `terminal_multiplexer_for_env`。
- 规模门禁（ADR 0090）：`agent/session/store.rs` 因自愈入口 5683 → 5704 行，基线同步该一条；
  回归测试落在新文件 `tests/session_row_heal.rs`，`sqlite_session.rs` 未增长。

## 取证（本次实测退回）

- `cargo check -p pony-agent-core --all-targets` → 0 error；
  `cargo test -p pony-agent-core --lib session` → 205 passed；定点
  `tests::full_store_write_keeps_sessions_dropped_from_latest_snapshot` → 1 passed；
  `tests/session_row_heal.rs` → 2 passed；`tui_terminal_restore` / `tui_startup_input_guard` / `tui_resume_cli` 全绿。
- `tools/tui-audit/check_shift_enter.py --bin target/debug/pony` → `pass=6 fail=0`，
  `a2.5` 用 herdr 真实环境（`HERDR_ENV=1`）得到 `push_flags=1`（DISAMBIGUATE only）。
- 真实库端到端（隔离 HOME + 真实库副本 431 MB，PTY 跑 `pony resume pa_01a1b69c-…`）：
  启动前 legacy 行 0 条 → 截屏出现「索引行曾丢失，已重建」与历史尾部正文（`↑102.7k ↓52.4k` 计数非零）
  → 退出后 legacy 行回写、`sessions` 73 → 74 行。真实库只读访问，未被本次验证改动。
