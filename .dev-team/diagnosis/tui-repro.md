# TUI 复现/验证报告：团队视图缺失 · 退出还原与 resume · 追问断流

- 角色：tui-repro（只读复现/验证；**未修改任何源码/配置，未做任何 git 写操作**）
- 时间窗：2026-11-05 17:04 – 17:14（Asia/Shanghai）
- 被测二进制：`/home/dm/pony-agent/target/debug/pony`，mtime `2026-11-05 16:47:59`，66,864,616 bytes（会话已带未提交改动，**未重新构建**，无全量 build/测试）
- 运行环境：tmux `pony-diag`，120×34；另用 `script(1)` 与自建 pty harness（`pty.fork`）抓原始字节
- 会话库隔离：为不干扰用户正在运行的 TUI（pid 917997，pts/1）与生产库，全部运行指向**副本**
  `XDG_DATA_HOME=/tmp/dsh-pony-tui-diag/xdg`（`PonyAgent/sessions.db` 为 `~/.local/share/PonyAgent/sessions.db` 的 17:05 快照，47 条会话）
- 证据目录：`/tmp/dsh-pony-tui-diag/`（tmux capture 文本 + `.raw` pty 原始字节 + harness 脚本）

---

## 1) 团队视图：不存在（主界面 / 斜杠菜单 / 帮助均无 roster、角色状态、子会话入口）

**结论：TUI 没有任何 agent-team 视图**。可用的只有 `/tasks`（Team 共享任务看板），它只是 `todo` 工具结果的汇总展示，不含角色（tester/reviewer…）、不含会话、不含状态、无法进入 teammate 子会话。

- 主界面（`01-main-screen.txt`）只有头部 `pony · pony-agent` + 模型名、logo、4 条命令提示、输入行、工作区行；无侧栏/无面板/无团队区。
- 输入 `/` 弹出的完整命令表（`02-slash-menu.txt`，共 11 条，与 `04-help.txt` 一致）：

```
/resume    选择并恢复历史会话
/provider  管理与切换模型提供商
/models    切换当前活跃模型
/thinking  设置思考强度 (Reasoning Effort)
/plan      切换/查看 Plan 面板（不限制工具执行）
/tasks     切换/查看 Team 任务看板（Ctrl+T）
/clear     清空当前消息上下文
/compact   压缩当前会话上下文
/help      查看所有支持的快捷指令
/update    检查更新与升级新版本
/quit      安全退出 TUI 界面
```

  源码一致（`crates/pony-agent-core/src/bin/tui.rs:717` 起 `AVAILABLE_COMMANDS`）：**无 `/team`、`/agents`、`/sessions`、`/subagent`**；全局快捷键只有 `Ctrl+T`（任务看板）、`F2`（鼠标捕获）等，无团队/子会话热键。
- `/tasks` 面板实际内容（`03-tasks-panel.txt` 原文）：

```
┌──────────────────────────────────────────────────────────────────────┐
│  Team 共享任务看板                        Tab 折叠/展开 · Esc 退出   │
│                                                                      │
│  当前没有任务；模型调用 todo 工具后会实时汇总到这里                  │
└──────────────────────────────────────────────────────────────────────┘
```

  即：**空态是「模型调用 todo 工具后会汇总」而非「团队角色/会话」**；进入 teammate 子会话、返回主会话的能力在 UI 层完全不存在（源码 `grep -n "teammate\|subagent" tui.rs` 仅命中 `/tasks` 描述与看板标题，无子会话路由）。

---

## 2) `/resume` 只列主会话，无 subagent 会话

- `/resume` 弹窗（`05-resume-picker.txt`）：标题 `会话 · 1/47`，列表项为「首条用户消息摘要 + 8 位 id 尾号」，例如：
  `为什么当前项目tui退出后，在终端中，输入任何内容都是一...  b1e4d476` / `当前项目是一个什么项目？  2d72fe33` / …（共 47 条，可 ↑↓ 选择）
- 对库副本 `select conversation_id from sessions` 全量核对：47 条前缀分布 `session`×42、`complex`×2、`probe`×2、`live`×1；**`subagent-*` / `teammate-*` 会话 0 条**（查询与结论见本节末命令）。即：既没有可切换的子会话列表，存储层也根本没有 teammate 子会话记录。
- 由此确认用户诉求「TUI 有团队视图（角色/会话/状态，可进入 teammate 子会话并返回主会话）」**当前完全未实现**。

```
$ python3 -c "..."  # 只读查询 /tmp/dsh-pony-tui-diag/xdg/PonyAgent/sessions.db
sessions cols: ['conversation_id', 'title', 'updated_at_ms', 'session_data']
total: 47
Counter({'session': 42, 'complex': 2, 'probe': 2, 'live': 1})
subagent-ish: []
```

---

## 3) 退出行为：三条路径均还原终端并打印 `pony resume <id>`，退出后输入正常

| 退出方式 | 证据 | 结果 |
|---|---|---|
| `/quit` | `06-quit-pane.txt`、`07-post-quit-typing.txt`、`runB-quit.raw` | 还原 + 提示 + 退出码 0 |
| `Ctrl-C` | `10-runC-after-ctrlc-pane.txt`、`runC-ctrlc.raw` | 还原 + 提示 + 退出码 0（`COMMAND_EXIT_CODE="0"`） |
| `SIGTERM`（`kill -TERM`） | `20-sigterm-pane.txt`、`21-post-sigterm-typing.txt` | 还原 + 提示 + 进程退出，随后 shell 输入正常 |

**提示行原文**（`06-quit-pane.txt`）：`继续该会话请运行：pony resume session-99f690fd`；Ctrl-C：`pony resume session-500d730f`；SIGTERM：`pony resume session-02396f59`。id 为 `session-<8hex>` 格式，可直接复制执行。

**退出后输入不复原的问题在本环境下不可复现**（已修）：`07-post-quit-typing.txt` 中退出后执行 `echo post-quit-input-ok` 正常回显、无 `ESC[...u` 乱码；`21-post-sigterm-typing.txt` 同样正常。

**PTY 原始字节证据**（`script(1)` 抓取，`runB-quit.raw` 尾部）：

```
...^[[?25l ^[[<1u ^[[<u ^[[>4;0m ^[[?1000l ^[[?1002l ^[[?1003l ^[[?1006l ^[[?1015l ^[[?2004l ^[[?1049l ^[[?25h
继续该会话请运行：pony resume session-c92b655a
```

顺序正确：**kitty 弹栈（两种写法）→ modifyOtherKeys 关 → 五级鼠标关 → bracketed paste 关 → 退出备用屏 → 显示光标 → 才打印提示**（提示落在主屏，未被备用屏吞掉）。

**kitty 能力终端的强验**（tmux 内 `supports_keyboard_enhancement()` 查询无应答，push 未发生；故用 `pty.fork` harness 应答 `ESC[?u` / `ESC[c` 模拟 kitty 终端）：

- 脚本：`/tmp/dsh-pony-tui-diag/kitty_timing2.py`；抓取：`runE3-kitty-timing.raw`、`runE4-kitty-timing.raw`
- 进入侧：`ESC[?1049h`、`ESC[?1000h ?1002h ?1003h ?1006h ?1015h`、`ESC[?2004h`、**`ESC[>9u`（kitty 键盘增强 push，flags=9）**
- 退出侧：**`ESC[<1u` + `ESC[<u`（pop 两种写法都发）**、`ESC[>4;0m`、`ESC[?1000l ?1002l ?1003l ?1006l ?1015l`、`ESC[?2004l`、`ESC[?1049l`、`ESC[?25h`，随后提示行
- push/pop 成对闭合 ⇒ 用户抱怨的「退出后输入乱码」的因果链在 kitty 类终端上已被还原串覆盖。
- 退出时延：`/quit` 后约 **0.03 s** 即关闭 pty 从端（EIO at t=7.09，quit at t=7.06），无挂起。
- harness 备注：只应答 kitty 查询而**不应答 `ESC[c`（DA1）**时，crossterm 的 `read_internal(PrimaryDeviceAttributesFilter)` 会阻塞（crossterm 0.28.1 `terminal/sys/unix.rs:239`），这是 harness 造成的假死（`runE2-kitty-timing.raw` 仅 317 字节），非 pony 缺陷。

**`pony resume <id>` 恢复历史已验证**（`11-runD-resume-screen.txt`、`12-runD-resume-history-top.txt`、`runD-resume.raw`）：
`./target/debug/pony resume session-2d72fe33` 启动即还原该会话历史（用户消息 + 工具结果行 + assistant 正文，含 `✓ 读取 (Read) · {...}` 反显），且 header 工作区还原为会话原 workspace `deepseek-harness`（不是启动目录），符合 `hydrate_state_from_session` 的 workspace 绑定决议。在该会话内 `/quit` 打出 `继续该会话请运行：pony resume session-2d72fe33`（`13-runD-quit-hint.txt`）——**id 保持同一，未漂移**。

---

## 4) 追问断流：稳定复现（3/3 失败，turn 无终态）

**复现步骤（可重放）**：

```
tmux new -d -s pony-diag -x 120 -y 34
cd /home/dm/pony-agent && export XDG_DATA_HOME=/tmp/dsh-pony-tui-diag/xdg
./target/debug/pony resume session-b1e4d476      # 用户原始会话（库副本）
# 等 12s 历史还原完成（14-runF-resumed-b1e4d476.txt）
# 输入框原样发送：继续，把 tester 和 reviewer 的收据补齐   （17:10:50）
```

- 现象（`15-runF-followup-t12.txt`）：12 s 后只多出两行

```
   ▌ 继续，把 tester 和 reviewer 的收据补齐
   ▌
   ✓ 读取 (Read) · 收集路径 / 的上下文。
```

  之后再无任何 assistant 文本、无错误提示、输入框可继续输入，右下角恒为 `上下文 0%`。
- 持续观察：`16-runF-t42.txt`（+42 s）、`17-runF-t72.txt`（+72 s）、`17-runF-t-late.txt`（+110 s）、`18-runF-final-before-kill.txt`（+150 s）**画面完全不变**；进程 `STAT Ssl+ / wchan futex_wait_queue`，`ss -tp` 无任何外连 ⇒ **不是等 API，是本地卡死**。
- 会话库（副本）事件取证（`22-stall-turn-events.txt`），该 turn `turn-8aed5cf7-…` 全部 6 条事件：

```
turn/start → context/observation → user/message("继续，把 tester 和 reviewer 的收据补齐")
→ step/start(step 0)
→ tool/call   name=Read  arguments={"description":"收集路径 `/` 的上下文。","limit":40,"lineCount":80,"path":"/"}
→ tool/result status=error  result={"error":{"code":"control_outcome_pending",…,"ok":false}}
              capabilityId="builtin:workspace_gather_context", requiresApproval=false,
              hostMediated=false, decisionSource="capability_registry", approvalMode="none"
```

  且：`turn/end` 事件数 **0**、`normalized_turns` 行 **0**、`session_turn_traces` 行 **0** ⇒ 该轮**永不终态**（不是「已完成但没渲染」，而是 `turn:suspended` 后再无恢复）。
- 更早两次同会话追问同样如此（同一副本库）：

```
turn-1c5533b9…  user="检查tester和reviewer的进展"   6 条事件，turn_end=0
turn-829bd610…  user="继续"                          6 条事件，turn_end=0
turn-7ee56f27…  （正常轮）                          727 条事件，turn_end=1, reason=completed
```

  ⇒ 失败率与用户描述一致：「只要追问就断」，而长任务首轮是正常完成的。

**`Read(path="/")` 与描述文案来自 planner，不是模型**：

- `crates/pony-agent-core/src/agent/planner.rs:385-397`：`format!("收集路径 `{}` 的上下文。", path)` + `"path": path, "lineCount": 80, "limit": 40` —— 与事件里的 arguments **逐字段一致**（`limit:40` / `lineCount:80` 是 planner 模板常量，模型不可能产出这种键组合）；工具名 `Read` 是 `model_tool_name("workspace_gather_context")` 的展示名。
- `planner.rs:236`：`let referenced_path = explicit_path.or_else(|| Self::infer_last_referenced_path(history));`，而 `infer_last_referenced_path`（`planner.rs:535`）从**历史消息**里倒序抽取路径。本次追问文本无任何路径，`/` 只能来自历史里残留的斜杠片段（如上一轮报告中的 `/quit`、`Read(path="/")` 等）⇒ **planner 用陈旧上下文「劫持」了追问，伪造出一次 `Read("/")`**。
- 该 planner 注入的调用随后返回 `control_outcome_pending`（`requiresApproval=false && hostMediated=false`，即**根本没有待宿主裁决的请求**），而运行期把 `control_outcome_pending` 当作「本轮挂起、等恢复」（`runtime/turn_sync.rs:167-173`、`turn_stream.rs:539-545`，`turn_control.rs:350` 的 `fallback_reason="control_outcome_pending"`）⇒ 无请求可解 ⇒ 永久 suspended。
- 因果链（复现结论）：**追问 → planner 注入 `workspace_gather_context`(path=`/`) → 结果 control_outcome_pending（无宿主请求）→ 本轮 suspended 且无恢复入口 → 用户看到「没有后续」**。
- 补充：该轮 `turn_events` 里 `step/start` 之后直接是 `step 3` 的 `tool/call`、`step 4` 的 `tool/result`，没有 provider/usage 与 assistant 事件 ⇒ 模型侧没有产出过任何回复内容。

---

## 5) 纪律声明 · 证据清单 · 未验证项

**纪律**

- 未修改任何源码/配置文件（`git status` 中 `crates/pony-agent-core/src/bin/tui.rs`、`docs/decisions/README.md`、`tests/tui_terminal_restore.rs`、`docs/decisions/0062-…md` 的改动**均非本任务产生**，为既有会话遗留）；**未执行任何 git 写命令**（只跑过 `git status` / `git diff` 只读命令）。
- 未重新构建（二进制 mtime 16:47:59 已含最新改动），未跑全量测试。
- 所有运行指向会话库**副本**（`XDG_DATA_HOME=/tmp/dsh-pony-tui-diag/xdg`），用户正在运行的 TUI（pid 917997）与生产 `~/.local/share/PonyAgent/sessions.db` 未被本任务写入（仅作 17:05 只读快照复制）。
- tmux 会话 `pony-diag` 已 `kill-session` 清理；被复现卡死的 pony 实例已终止；证据文件全部保留。

**证据文件（`/tmp/dsh-pony-tui-diag/`）**

| 文件 | 内容 |
|---|---|
| `01-main-screen.txt` | 主界面（无团队面板） |
| `02-slash-menu.txt` | `/` 完整命令表（11 条，无 team/agents） |
| `03-tasks-panel.txt` | `/tasks`「Team 共享任务看板」空态原文 |
| `04-help.txt` | `/help` 命令列表 + 网关 |
| `05-resume-picker.txt` | `/resume` 会话列表（1/47） |
| `06-quit-pane.txt` / `07-post-quit-typing.txt` | `/quit` 后还原 + 提示 + 退出后输入正常 |
| `08-runB-start.txt` / `runB-quit.raw` | `script` 包裹下的启动画面 / `/quit` 全量原始字节 |
| `10-runC-after-ctrlc-pane.txt` / `runC-ctrlc.raw` | Ctrl-C 路径 |
| `11-runD-resume-screen.txt` / `12-runD-resume-history-top.txt` / `13-runD-quit-hint.txt` / `runD-resume.raw` | `pony resume session-2d72fe33` 历史还原与 id 保持 |
| `14-runF-resumed-b1e4d476.txt` | 复现前：`resume session-b1e4d476` 历史还原 |
| `15-runF-followup-t12.txt` | **追问断流关键捕获**（只剩一行 `✓ 读取 (Read) · 收集路径 / 的上下文。`） |
| `16-runF-t42.txt`、`17-runF-t72.txt`、`17-runF-t-late.txt`、`18-runF-final-before-kill.txt` | 停滞 42/72/110/150 s 画面不变 |
| `22-stall-turn-events.txt` | 该 turn 6 条事件全文 + `turn_end=0` / 无 normalized 记录 + 同会话 3 次追问对比 |
| `runF-followup.raw` | 复现全程原始字节 |
| `runE3-kitty-timing.raw`、`runE4-kitty-timing.raw` | kitty 终端模拟：`>9u` push 与 `<1u`/`<u` pop 成对；退出时延 0.03 s |
| `20-sigterm-pane.txt` / `21-post-sigterm-typing.txt` | SIGTERM 路径还原与提示 |
| `kitty_sim.py`、`kitty_sim2.py`、`kitty_timing.py`、`kitty_timing2.py`、`dbg.py` | pty harness 脚本（复现用，非产品代码） |

**已知未验证项**

1. **Windows 行为**：未验证（`restore_terminal_modes` 的 `#[cfg(not(unix))]` 分支走 crossterm 命令，本机为 Linux）。
2. **真实 kitty/WezTerm 物理终端**：只用 pty harness 模拟应答，未在真机 kitty/WezTerm 窗口验证（tmux 内 `supports_keyboard_enhancement()` 无应答，push 分支需 harness 才触发）。
3. **无 API key / 断网边界**：未构造；本次追问断流发生在**已有可用网关**（`http://10.42.0.67:8080`）且无外连的条件下，属本地卡死，与网络无关；provider 侧错误路径（401/限流）未覆盖。
4. **团队视图**：只验证「不存在」，未评估实现方案成本（`/tasks` 看板与 team roster 是不同数据源）。
5. **`control_outcome_pending` 的完整恢复语义**：只确认「无宿主请求时该轮永不终态」这一现象与代码注释口径一致，未深入 `turn_control.rs` 的 CAS/所有权路径（属诊断另一任务域，见 `trace.md`/`team-audit.md`）。
6. 采集为 120×34 单一尺寸；窄终端/小窗口下的布局退化未覆盖。
