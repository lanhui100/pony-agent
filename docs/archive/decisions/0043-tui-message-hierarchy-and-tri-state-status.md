# ADR 0043：TUI 消息区层级弱化、代码块材质与三态状态词

Status: implemented

## 背景

用户在实测 `pony` TUI 时提出三点：

1. **中间态抢戏**：思考/工具调用行与正文同亮度，阅读时第一眼落在过程而不是结论上；
2. **Agent 正文观感粗糙**：代码块与正文同材质（仅靠 `▏` 竖线区分），长答案里代码段落散在画布上，缺少「区块」感；系统提示层也缺少对「长篇大论」的硬约束；
3. **底部状态词失真**：流式正文阶段显示 `写入中`，把「模型在生成自然语言」误述为「写入文件」，且与「思考中 / 工具」并列时语义不齐。

前一轮 wave-39/41 已把指标行、composer 几何收边（ADR 0042），本条只处理「正文区层级 + 状态词」，不减信息、不加装饰。

## 决策

1. **中间态降权（工具行 / 思考行）**
   - 工具行：运行中 spinner 用 `ACCENT_DIM`（不再用 `ACCENT`）；完成 ✓ 用 `FAINT`（不再用 `SUCCESS` 绿）；动作名按 `MUTED`（调用中）/ `FAINT`（已结束）；参数与结果统一 `FAINT`。**失败是例外**：✕ 与动作名回 `DANGER`，错误不能被弱化。
   - 判定原则：只有「结论、错误、待用户确认」三类信息可以满对比；过程信息一律 ≤ `MUTED`。
2. **代码块材质化（终端字号不可变，改用「底 + 字重落差」表达次级层级）**
   - **整块一条色带**：语言标签行（`json` 等）与所有代码行共用同一条 `theme::CODE_FILL`（`Rgb(23,22,20)`，比 `SURFACE` 更贴画布）色带，块内不出现任何画布底色缝隙；
   - **取消发丝竖线**：不再用 `▏` 区分代码区，色块边界本身即语义；色带从轨道（2 列）之后起排，**右端内缩 2 列**（`CODE_BAND_INSET`）不贴内容列右缘；
   - 色带内部左内边距 2 列（`CODE_BAND_PAD`）；超出宽度的代码按显示宽度折行，折行同样继承整块底色；
   - 代码正文 `MUTED + Modifier::DIM`；语言标签保持 `FAINT`；
   - 行内代码前景由 `INK` 降到 `INK_SOFT`，底色仍为 `CODE_FILL`。
3. **三态状态词（状态机语义收敛）**
   - `RuntimeMode::Thinking → 思考中`、`RuntimeMode::Responding → 生成中`、`RuntimeMode::ExecutingTool → 执行 · <工具名>`；`runtime_status_display()` 同步为 `思考中... / 生成中... / 执行: <工具名>...`。
   - **`写入中` 一词从 TUI 消除**：它把「模型生成」误指为磁盘写入；写文件本身只是 `执行` 态里的一种工具。
   - 状态词配色：`思考中`/`执行 · x` = `FAINT`，`生成中` = `MUTED`（唯一「读者正在等的内容」略高半档），`完成` = `SUCCESS`、`失败` = `DANGER` 不变；活动行 spinner 由 `ACCENT` 降为 `ACCENT_DIM`。
4. **系统提示层加硬约束（与 TUI 共享的 `BASE_SYSTEM_PROMPT`）**
   - 新增 `Concise by default`：禁止冗长前言、寒暄、谄媚与复述式叙述；
   - `Milestone conciseness` 补一句「不要叙述显而易见的中间步骤」；
   - `Code clarity`：不为一次性操作造抽象，三行重复优于过早抽象（对齐 claude-code 的 codeStyle 口径）；
   - 原有工程契约（read-before-write / physical receipts / anti-entropy / `todo_write`）逐字保留。
5. **命令菜单跟随命令表**：菜单行数上限由 7 提到 8，`/thinking` 加入后不再把末尾 `/quit` 挤出可视区。
6. **移除助手消息前的 `pony` 字标**：每轮 assistant 正文前原本都渲染一行骨色 `pony` wordmark，与 header 已有的品牌标识重复，且把正文起点压低一行。角色改由「用户块材质 + 轨道 + 留白」承担：turn 之间统一 2 行留白（工具行回到正文由 `push_blank` 保持 1 行）。header 的 `pony · <workspace>` 保留为唯一品牌锚点。

## 备选方案与取舍

- **保留工具行绿 ✓ / 金色 spinner**：`SUCCESS` 绿在暗底上是「完成」的品牌色，但每轮多次工具调用会连续刷绿点，视觉权重高于结论；降为 `FAINT` 后错误色成为唯一彩色信号，反而更可辨。
- **代码块画整块边框卡片**：违反 ADR 0038「永不画整块边框卡片」的语汇；底色带 + 缩进已足够形成区块。
- **保留 `▏` 发丝竖线 + 逐行底色**：竖线与底色叠加是两套边界语汇（线 + 面）互相打架，且标签行悬空在色块之外；改为单一面状色带后层级更干净。
- **用 `Modifier::DIM` 单独表达「更小」**：DIM 在部分终端不可靠，故与 `MUTED` 叠加使用，去掉 DIM 仍可读。
- **保留 `写入中` 另加 `生成中`**：两个词会在同一阶段竞争表达，直接替换更干净。
- **保留 `pony` 字标、仅将其降为 `FAINT`**：仍然每轮多占一行且重复 header 品牌信息；「更弱」不等于「该留」，故整行删除。**把字标换成 `Agent` 文本标签**：与已删除的 `你` 角色标签同类（ADR 0042 已确立「用材质而非文字标签表达角色」），不再引入。

## 后果

- `crates/pony-agent-core/src/bin/tui.rs`：工具行字形/配色、代码块渲染、`status_word()`、`runtime_status_display()`、菜单行数上限、assistant `pony` 字标移除（`kicker` 闭包删除，turn 间距改由 `push_gap(2)` / `push_blank` 表达）。
- `crates/pony-agent-core/src/agent/context.rs`：`BASE_SYSTEM_PROMPT` 输出约束段（桌面端同样受益，非 TUI 专属）。
- 既有测试适配：`tests/tui_acceptance.rs`（`Responding` 文案、命令表条数）、`tests/tui_render_layout.rs`（流式活动行词）、`tests/tui_dialog_wave39.rs`（指标行禁用词表），`tools/tui-audit/checks.py` / `check_shift_enter.py` 的断言词同步为三态。
- 证据：`tools/tui-audit/run_audit.sh --phase after` 全 18 场景 369 项断言通过；截图见 `.dev-team/evidence/tui-polish/{baseline,after}/`。
- 已知基线问题（不在本条处理）：`tests/tui_render_layout.rs` 中 6 条 wave-39 几何断言（header 发丝线、光标列、CJK 折行等）在本次改动前即为红，属既有基线债。

## 关联

- ADR 0038（TUI 视觉语汇：发丝线 / 弱化层级）
- ADR 0042（composer 收边与底部指标行，`写入中` 的首次出现处）
