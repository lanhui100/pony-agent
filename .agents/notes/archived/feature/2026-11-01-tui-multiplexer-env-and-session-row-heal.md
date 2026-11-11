# Agent Note: herdr 复用器变量名对齐 与「整包写盘不再隐式删会话 + resume 自愈重建」

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0094-tui-multiplexer-env-and-session-row-heal.md` 之归档快照，仅供溯源。

## 背景

两个缺陷同源自一次真实用户现场（herdr 0.7.5 pane 中运行 `pony`），互相独立但都在「退出 TUI → 复制
`pony resume <id>` → 新 shell 粘贴」这条链路上爆出。

### 缺陷 1：Enter 不提交，行尾多出一个 `u`

`/proc/<pid>/environ` 实测：herdr 拉起的 shell 只注入 `HERDR_ENV=1`、`HERDR_PANE_ID`、
`HERDR_SOCKET_PATH`、`HERDR_TAB_ID`、`HERDR_WORKSPACE_ID`，**没有**名为 `HERDR` 的变量。

ADR [0075](0075-tui-exit-restore-hardening.md) 的复用器清单是 `HERDR`/`TMUX`/`ZELLIJ`/`STY`/`ABDUCO`，
于是 herdr 被判成「直连终端」⇒ push `DISAMBIGUATE|REPORT_ALL_KEYS`(`CSI > 9 u`)。herdr 不解析 kitty
键盘协议、pop 也不保证透传，退出后按键仍按「全键位上报」编码：Enter 变成 `CSI 13u`，bash/readline
把它当 `\x07` + `u` 回显 ⇒ 「回车不提交，行尾多一个 `u`」。PTY 复现：向 bash 发 `CSI 13u`，输出 `\x07u`。

### 缺陷 2：`pony resume <id>` 说「没有该会话」

真实库 `~/.local/share/PonyAgent/sessions.db` 证据：`pa_01a1b69c…` 的 `normalized_sessions`
（3 轮 / 6 条消息）、`normalized_messages`、`normalized_turn_traces`(3)、`normalized_history_nodes`(3)、
`cursor`、`turn_events`(1266) **全在**，唯独 legacy `sessions` 那行 blob 不见了。而
`SessionStore::list_sessions()` / `resolve_resume_target()` 只读 legacy 行 ⇒ `existed=false` ⇒
TUI 打印「尚无历史记录，已沿用该 id 开始新会话」。

能删掉这一行的只有 `SqliteSessionBackend::write_full_store` 的兜底逻辑：整包写盘时把「内存快照里
没有的会话」逐行 DELETE。多个 TUI / 桌面端 / `ponycli` 并发写同一库时，任何一份启动时装好的旧快照
在整包写盘（工作区引导回写、元数据回写、历史图修复）都会**删掉别的实例新建的会话**。旁证：三分钟内
`sessions` 表从 72 行变 71 行且最新行换了 id；全库 49 个会话只剩规范化行。

## 决策

1. **复用器清单以「真实现场变量」为准**：`TERMINAL_MULTIPLEXER_ENV_KEYS` 扩为
   `HERDR`/`HERDR_ENV`/`HERDR_PANE_ID`/`HERDR_SOCKET_PATH`/`HERDR_TAB_ID`/`HERDR_WORKSPACE_ID`/
   `TMUX`/`ZELLIJ`/`STY`/`ABDUCO`，并改为 `&[&str]`（清单会继续增长，不锁数组长度）。herdr 下因此
   只 push `DISAMBIGUATE`：Shift+Enter 照常可用，pop 丢失时 Enter 仍是 `\r`。
   `tools/tui-audit/check_shift_enter.py` 与 `tests/tui_startup_input_guard.rs` 内嵌的清单同步扩写
   （三处同口径，任一处漏写都会让审计在 herdr 里报假失败）。
2. **整包写盘只写不删**：`write_full_store` 去掉「对齐内存快照」的 DELETE（含 `session_turn_traces`）。
   删除会话的唯一合法路径是显式 `remove_session`，它在同一事务里删 legacy 行、trace 与**规范化行**
   （新增：否则自愈逻辑会把用户已删除的会话复活）。
3. **resume 按需自愈**：新增 `SessionBackend::load_normalized_session(id)`（默认 `None`）+
   `SessionStore::recover_session_from_normalized(id)` + `HostControlPlane::recover_session_from_normalized`。
   `pony resume <id>` 在 `existed=false` 时先尝试从规范化表重建该会话并回写 legacy 行，成功后照常
   hydrate，并在状态行与首条 System 消息里如实告知「索引行曾丢失，已重建」。
   只在**显式 resume** 时触发，不做启动期全量合并。

## 影响

- **用户可见行为**：herdr 下退出 TUI 后 Enter 恢复正常；被误删索引行的会话可用提示里的命令恢复出
  真实历史；多开实例不再互相抹掉会话。
- **数据面**：`sessions` 表不再被整包写盘缩减；显式删除同时清三处，不会复活。
- **契约面**：`SessionBackend` 新增默认方法（非破坏性）；`TERMINAL_MULTIPLEXER_ENV_KEYS` 类型由
  `[&str; 5]` 改 `&[&str]`（内部常量，唯一消费者是 `terminal_multiplexer_for_env`）。
- **规模门禁（ADR 0090）**：`agent/session/store.rs` 因本 ADR 增 21 行（自愈入口），
  `scripts/file-size-baseline.json` 中该文件的 `file-lines` 条目 5683 → 5704，diff 仅此一条、人工核对后接受；
  回归测试落在新文件 `crates/pony-agent-core/tests/session_row_heal.rs`，`sqlite_session.rs` 因此未增长。
- **不解决**：规范化表与 legacy 行长期双写的一致性债，另由
  [0093 会话存储事件溯源重构](proposed/0093-session-storage-event-sourced-refactor.md)（proposed）承接。

## Alternatives considered

- **A：只加 `HERDR` 的别名映射（把 `HERDR_ENV` 当特例硬编码在 `terminal_multiplexer()` 里）**。
  落选：判定点分裂成「清单 + 特例」，下个复用器改变量名时又要改两处；清单本身就是分级决策的输入。
- **B：在 herdr 下完全不 push 键盘增强**。落选：Shift+Enter 会退化成裸 Enter（＝直接发送），
  用删功能换稳定性，与 ADR 0075 的取舍一致地不可接受。
- **C：退出时无条件追加 `CSI = 0 u` 置零（加强 ADR 0075 的还原串）**。落选：置零已在还原串里，
  本缺陷的根因是 push 面过大（`REPORT_ALL_KEYS`），置零只能事后补救、不能阻止退出前的乱码。
- **D：整包写盘改为「合并写」（读出库内全集再 upsert）**。落选：那只是把删除换成「读全库 + 写全库」
  的更大事务，跨实例的丢更新仍在；写盘语义应止步于「把我知道的写下去」。
- **E：启动时把规范化表里所有会话都合并回内存**。落选：会把用户显式删除后仅剩规范化行的残留自动
  复活，与「删除要留得住」冲突；故收敛为显式 resume 才自愈。
- **F：`pony resume <id>` 找不到时直接报错退出**。落选：退出提示里的 id 常来自未落库的会话，
  严格校验会把提示变成假承诺（既有注释已记录该取舍）。

