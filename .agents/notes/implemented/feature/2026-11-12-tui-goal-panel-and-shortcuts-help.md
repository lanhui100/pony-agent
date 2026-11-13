# Agent Note: TUI 目标面板 (Ctrl+G) 与快捷键帮助覆盖层 (Ctrl+H)

Status: implemented

- 日期：2026-11-12
- 状态：implemented
- 作用域：Rust TUI（`crates/pony-agent-core/src/bin/tui/**`）
- 对照记录（桌面端）：`.agents/notes/implemented/feature/2026-11-12-goal-panel-shortcuts-and-help-dialog.md`

## Context

桌面端已在同一波次落了「Ctrl+G 目标面板 + Ctrl+H 快捷键帮助」（Vue 3 侧栏面板 + Dialog 弹窗），
而 TUI 侧的 `/goal` 虽然早已有 core 语义真源（`pony_agent_core::agent::goal_command`），但：
1. 目标状态只能靠底部横幅的一行摘要感知，看不到 objective 原文、阶段、轮次与已停止原因；
2. 快捷键散落在 `tui.rs`、`input.rs`、`queue.rs` 三处分支里，用户没有任何「就地查阅」的入口，
   新加的 Ctrl+O / Ctrl+P / Ctrl+T 等在 `/help` 文本里只有零散提及；
3. 桌面端与 TUI 是同一产品语义的两个前端，「同一功能在 TUI 无处可达」本身就是可见性缺口。

同时 TUI 与桌面端存在一个**不可消除的结构差异**：桌面端有右侧栏（`workspace-panels` 的
`plan | roster | tasks | goal` 互斥面板），TUI 只有一块由消息流独占的正文区，没有旁挂列的上下文。

## Decision

1. **目标面板采用居中覆盖层，不做侧栏**：TUI 没有侧栏这一布局概念，任何「常驻第 4 列」
   都要重写整条渲染管线（消息区宽度预算、markdown 表格折行、图片协议区域）。
   覆盖层复用既有 `draw_panel` + `theme::SURFACE` 口径（与设置面板、空态引导同一套），
   打开时 `queue::is_modal_mode` 判为模态、`is_ui_idle_for_dispatch` 拒绝派发——
   即「不占常驻空间，但打开时独占按键与派发」。
2. **面板是只读投影，不产生任何状态写入**：`tui/goal_panel.rs::render_lines` 只做
   `TuiState → Vec<String>` 的纯投影（objective → 阶段 → 轮次 → 已停止原因 → 操作提示），
   暂停/恢复/编辑/清空仍只有 `/goal` 一条路径。理由：goal 状态机与回执文案的真源在 core
   （`agent/goal_command.rs` 的 `parse_goal_command` / `execute_goal_command`），
   面板里再造一套按钮式写入口，等于**第二套状态机**。
3. **快捷键清单是 TUI 的静态唯一真源**：`tui/shortcuts.rs::SHORTCUT_SECTIONS`（4 组 26 条）
   承载「用户可见清单」，`tui.rs` 里不允许再写一份只给帮助面板用的字面量表；
   键位判定收敛为纯函数 `is_shortcuts_help_shortcut` / `is_goal_panel_shortcut`，
   口径为 `modifiers == CONTROL && code == Char('h' | 'g')`（严格相等，不用 `contains`）。
4. **Ctrl+H 与退格不冲突（实证，不是推测）**：crossterm 0.28.1
   `src/event/sys/unix/parse.rs:106` 的 `c @ b'\x01'..=b'\x1A'` 分支把 0x08 解析为
   `KeyCode::Char((c - 0x1 + b'a') as char) + KeyModifiers::CONTROL`（即 `Char('h') + CONTROL`）；
   而退格走 `\x7F`——同文件 `:103`（普通解析分支）与 `:560`（kitty 协议分支）
   都映射为 `KeyCode::Backspace`。两条路径的字节域不相交，故 Ctrl+H 可安全用作覆盖层开关；
   若将来改成判定 `0x08` 或 `contains(CONTROL)`，两者会互相污染。
5. **与桌面端的一致性口径：语义同源、清单元数据各自维护**：
   - 同源部分：`/goal` 的语法、状态机、回执文案真源在 core（Rust），
     TUI（`tui/goal_command.rs`）与桌面端（`src-tauri/src/lib.rs` 的 `goal_command`）
     都只做「取参数 → 调 core → 投影到本端」，因此**不存在两份 goal 语义**；
   - 各自维护部分：快捷键清单是**呈现层资产**，Rust（`SHORTCUT_SECTIONS`）与
     TS（`src/lib/chat/keyboard-shortcuts.ts` + `src/components/ShortcutsHelpModal.vue`）
     各一份，跨语言没有共享真源。这是刻意接受的重复。
   - **不采用跨进程清单同步**（例如 TUI 启动时向 GUI 进程拉清单、或抽一个 JSON 清单产物
     由两端构建期读取）：清单是**每端键位各自的物理事实**——TUI 有 `Ctrl+Z` 挂起、
     `Alt+Enter` 换行、`Ctrl+↑` 返回主会话，桌面端有 `Cmd` 变体与浏览器级按键拦截，
     两端条目本就不同集合。做同步只能引入「一端删除条目导致另一端渲染空面板」的耦合，
     而收益仅是措辞统一；成本（新 IPC 契约 + 双向版本兼容 + 两端 CI 交叉依赖）
     明显大于收益。真正需要同源的是**语义**（goal 命令），那已经同源了。
6. **防漂移测试放在缝隙而非重复实现者单测**：新增
   `tui/tests/shortcuts_help_tests.rs` 只锁跨模块不变量——
   `is_modal_mode` 仍认两个新模式、（清单 token ↔ 判定函数）仍一一对应、
   空态仍给 `/goal` 入口、`/help` 描述仍含「快捷键」。
   条目内字段级投影由各自模块的内联单测负责（两套测试分工明确，不复制断言）。

## Alternatives considered

1. **用 `/help` 的系统消息流替代 Ctrl+H 覆盖层**：*未采纳*。消息流是被动阅读面：一按就
   插入一段长文本污染会话历史，且无法原地关闭；26 条清单会让后续对话被推远，还会与
   「/help 只列命令」的既有心智冲突。覆盖层可 Esc 关闭、不写历史，更贴合「即时查阅」。
2. **用 `Ctrl+?` 或 `F1` 替代 `Ctrl+H`**：*未采纳*。`Ctrl+?` 在不同终端的字节编码不一致
   （部分终端与 `Ctrl+/`、`0x7F` 混叠，恰好撞上退格域）；`F1` 在 tmux/screen 与
   macOS 键盘上常被系统或终端占用，且 F 区键已被 F2（鼠标捕获）、F3（思考强度）分割，
   再占用会破坏「F 区 = 显示类开关」的既有分组。`Ctrl+H` 语义（help）最自然，
   且已用 crossterm 解析证据确认与退格不冲突。
3. **把目标面板做成常驻的「第 4 列」**：*未采纳*。TUI 正文区宽度预算按单栏计算
   （markdown 表格折行、代码块、图片协议区域都依赖整宽），加常驻列等于重写渲染管线；
   且目标状态是低频查询对象，常驻收益远低于它挤占的对话空间。覆盖层 + 底部横幅摘要，
   兼顾「随时可见」与「不占空间」。
4. **让目标面板承担写操作（暂停/恢复按钮）**：*未采纳*。那会造出第二套状态机与第二套
   回执文案，且 TUI 的写入口必须与 core 的 revision/租约语义对齐；`/goal` 已是唯一写路径，
   面板只做只读投影 + 底部提示命令清单。

## Machine Evidence

- `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --bin pony shortcuts_help`（exit 0，7 passed）
- `cargo check --manifest-path crates/pony-agent-core/Cargo.toml --bin pony`（exit 0）
- `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --bin pony`：本记录新增 7 条后 **147 passed / 0 failed**（exit 0）
- 新增 `crates/pony-agent-core/src/bin/tui/tests/shortcuts_help_tests.rs`（跨模块不变量，不重复内联单测）
- 清单真源：`crates/pony-agent-core/src/bin/tui/shortcuts.rs`（`SHORTCUT_SECTIONS`，4 组 26 条）
- 模态看守：`crates/pony-agent-core/src/bin/tui/queue.rs::is_modal_mode` 已含 `ShortcutsHelp | GoalDetail`
- crossterm 解析证据：`~/.cargo/registry/src/index.crates.io-*/crossterm-0.28.1/src/event/sys/unix/parse.rs:106`（`0x01..=0x1A` → `Char + CONTROL`）、`:103` 与 `:560`（`0x7F` → `Backspace`）
- 门禁：`npm run adr:check` exit 0、`node scripts/backlog.mjs lint` exit 0（B131 归档后：活跃 12 / 归档 62）、`node scripts/backlog.mjs doctor` exit 0（ok:true, issues:[]）
- backlog：`B131` start → resolve，已归档至 `backlog/archive/tasks-2026.jsonl`
