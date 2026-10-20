# TUI 真实终端（tmux）验证凭证

本目录记录在**真实 tmux 会话**里运行 `target/debug/pony` 的验证结果，以及把
`tmux capture-pane -e` 的 ANSI 画面渲染成 PNG 的脚本。所有截图都是真实进程渲染，
不是设计稿。

## 1. 复现命令

```bash
# 1) 编译
cargo build --bin pony

# 2) 单张截图（真实 tmux 会话驱动）
./.dev-team/evidence/tui-tmux/capture.sh shot-plan .dev-team/evidence/tui-tmux \
  --key "/plan" --key "Enter" --shot 03-plan

# 3) ANSI 画面 → PNG（CJK 按 East-Asian Width 占 2 格，与 TUI 自身 char_width 同构）
python3 .dev-team/evidence/tui-tmux/ansi2png.py <file>.ansi <file>.png
```

## 2. 截图清单与对应能力

| 文件 | 画面 | 证据类型 |
| :-- | :-- | :-- |
| `01-home.png` | 首屏、发丝线、Composer、指标行 | 真实 tmux |
| `02-command-menu.png` | 打入 `/` 后的命令面板（含 `/plan`、`/tasks`） | 真实 tmux |
| `03-plan.png` | `/plan` 计划面板（无计划时的空状态） | 真实 tmux |
| `04-tasks.png` / `05-tasks-closed.png` | `/tasks` 任务看板打开 / `Esc` 关闭 | 真实 tmux |
| `06-ask-dialog.png` | **真实 Ask 挂起**后自动弹出的问答对话框 | 真实 tmux + 真实运行时挂起 |
| `07-ask-answered.png` | Enter 回答后对话框关闭、工具行转 `✓` | 真实 tmux + 控制面 CAS 消费 |
| `08-tasks-real.png` | 任务看板显示 `TodoWrite` **真实落库**的 3 条任务与 1/3 汇总 | 真实 tmux + 真实 todo 存储 |
| `09-goal-metrics.png` | 指标行显示 `create_goal` **真实落库**的 Goal 与 `[0/6]` 轮次 | 真实 tmux + 真实 goal 存储 |
| `10-plan-real.png` | 无计划时会话内 `/plan` 的空状态（不再整屏空白） | 真实 tmux |
| `11-goal-phase.png` | Goal 相位：`Goal · <目标> [0/6] (进行中)` 左对齐横幅 | 真实 tmux + 真实 goal 存储 |
| `12-metrics-phase.png` | 指标相位：`↑1.7k ↓4 · 缓存 0% · 上下文 0%` 右对齐数值 | 真实 tmux |
| `13-ctrl-t-board.png` | **`Ctrl+T` 直接呼出**任务看板（与 `/tasks` 同路径） | 真实 tmux |
| `17-ask-inline-polished.png` | Ask **内联**面板打磨版（发丝线分隔、`▏`焦点行、说明紧随标签） | 真实 tmux + 真实挂起 |
| `18-ask-inline-custom.png` | 自定义输入聚焦态：光标块 `▌` + 「键入后 ↵ 提交」 | 真实 tmux |
| `19-ask-inline-answered.png` | 提交后回到普通输入态，工具行转 `✓` | 真实 tmux |
| `21-code-block-polished.png` | **深赤陶强调色 + VS Code 语法着色陶土代码块**（上下呼吸留白 + 3列内边距） | 真实 tmux |
| `22-code-block-padding.png` | **代码块内部上下陶土暗底内边距**（`push_code_band_padding` 顶部+底部各 1 行） | 真实 tmux |
| `23-code-block-external-spacing.png` | **代码块外部下方空行修复**（修复后代码块下方拥有清晰的 1 行外呼吸留白） | 真实 tmux |

## 3. Ask 挂起链路的机器凭证

`tui-runtime.log`（隔离 HOME）中的关键行，证明是**真实运行时挂起**而非测试构造：

```
[pony-runtime] turn:capability-resolved capability_id=builtin:ask_user kind=tool mode=direct_tool_call
[pony-runtime] turn:ask-suspend-stream hop=1 request=control-000000000000 kind=Interaction call_id=call_mock_1 bound_run=None
```

`kind=Interaction` 即控制面 `PendingControlRequest`；TUI 主循环按 500ms 粒度调用
`control_plane.list_pending_asks(Some(session_id))` 取到它并弹出对话框；Enter 后
`answer_ask(request_id, version, answer)` 做 CAS 消费，工具行随即变成
`✓ 提问 (Ask) · ...`。

## 4. 上游是如何被替换的（不是伪造 TUI 状态）

真实模型网关当前对「工具调用后的 followup 回放」返回
`400 messages.1.content.0.tool_use.id: Field required`（见 §6），因此无法用真实模型
稳定触发工具挂起。为保证验证**仍然走真实运行时 + 真实控制面**，只把最外层
OpenAI 兼容网关替换为本地 stub：

- `mock_gateway.py`：直接返回一个工具调用（`Ask`）；
- `mock_script_gateway.py` + `plan-script.json`：按脚本依次返回
  `TodoWrite` → `create_goal` → 文本收尾。

TUI 侧全程使用隔离配置：

```bash
HOME=/tmp/pony-mock-home4 XDG_CONFIG_HOME=/tmp/pony-mock-home/.config target/debug/pony
# providers.json 的 base_url 指向 http://127.0.0.1:<port>/v1
```

即：**工具执行、挂起、待决请求、CAS 消费、todo/goal 落库、面板渲染全部是真实代码
路径**，只有 LLM 的 token 生成被 stub 替代。

## 4.1 Goal 与指标行的交替轮播（波次 44）

Goal 原先被拼在指标行右侧，长目标会把 token/缓存/上下文挤掉。现在改为**时间片轮播**：

- `GOAL_BANNER_HOLD_MS = 6000`、`METRICS_HOLD_MS = 8000`（`goal_banner_phase()` 单点定义）；
- Goal 相位整行**左对齐**成横幅（`Goal · <目标> [轮次/上限] (阶段)`），其中 `Goal` 标签
  单独加重（`BOLD` + 比正文亮一档的 `MUTED`），作为横幅唯一强调点；指标相位保持右对齐数值；
- 相位边界由主循环精确重绘（`cap_poll_ms` 对齐到下次切换），空闲时不空转。

实测采样（1s 粒度）：

```
1..5  GOAL     6..13 METRICS     14..18 GOAL   ...
```

快捷键：`Ctrl+T` 呼出/收起任务看板（面板内再按一次也关闭）。

## 4.2 Ask 改为信息流内联（不弹窗）+ 呼吸提醒

弹窗会遮挡正文、打断阅读。现在 Ask 面板贴在**正文底部、色带之上**：

- 顶部一条 `▁` 发丝线（HAIRLINE 色）把面板钉在信息流底部，不再悬浮；
- 标题 `▌ ◇ 提问  问题`（`提问` 用 ACCENT BOLD，问题用 INK BOLD），脉冲 `◇/◆` 每 450ms 交替；
- 选项行 `▏ ● 确认  继续执行该请求`：焦点行整行 `SURFACE_SEL` + INK BOLD，
  说明文字紧跟在标签后（不再右对齐拉满一行），焦点行说明提亮为 INK_SOFT；
- 单选 `●/○`、多选 `▣/▢`，marker 与标签之间固定一空格，列宽严格对齐；
- 自定义输入聚焦时尾部带光标块 `▌`，右侧提示「键入后 ↵ 提交」，提示行同步切换为
  「键入文本 · ↵ 提交 · esc 取消」；未聚焦时行文案为 `自定义输入…`；
- 提示行按状态给最短一组：单选「↑↓ 移动 · ↵ 确认 · esc 取消」/ 多选「↑↓ 移动 · space 勾选 · ↵ 提交 · esc 取消」；
- 问答期间色带只留一行 `◇ 等待你的选择…`，不再重复键位说明；
- 高度 `inline_ask_height()` 自适应（宽终端 ≤12 行、窄终端 ≤8 行），永远留 1 行给提示；
- 运行时内部 id（`builtin:ask_user`）不作为分组标签展示（表现层过滤，映射契约不变）。

权限审批（Permission Request）仍是弹窗，本轮未改动。

## 4.3 陶土强调色与 VS Code 风格代码块（波次 45）

1. **强调色升级为深赤陶色（Terracotta）**：
   - 行内 `**强调文字**` 与标题重点使用明艳饱满的深赤陶色（`#E8805C` / `theme::EMPHASIS`），打破灰度单调并凸显层级；
   - 角色标识与结构色同步选用暖赤陶色（`theme::ACCENT` / `#D67250`）。
2. **代码块呼吸留白与增强陶土暗底**：
   - 代码块与上部、下部正文段落之间各保证独立的 1 行外呼吸留白（`push_blank` / `lines.push(blank())`），修复了此前因去重逻辑导致代码块下方外部空行丢失的问题；
   - 代码块内部增加**顶部与底部各 1 行带有陶土暗底色的内边距**（`push_code_band_padding`），文字不再紧贴代码容器上下边缘；
   - 代码块背景采用深沉温暖的深陶土暗黑底（`#241916` / `theme::CODE_FILL`），质感高级内敛；
   - 内部左右内边距扩充至 **3 列**（`CODE_BAND_PAD = 3`），形成如同原生编辑器的舒适包裹感。
3. **VS Code 经典语法着色**：
   - 语言围栏行显示为带有前缀的暗绿斜体注释 `// rust`；
   - 词法识别关键字（紫色 `#C586C0`）、控流词（粉红 `#E06C75`）、字符串（橙粉 `#CE9178`）、注释（草绿 `#6A9955`）、类型结构体（青碧 `#4EC9B0`）、数字数值（浅绿 `#B5CEA8`）、变量标识符（天蓝 `#9CDCFE`）以及符号属性宏（亮蓝 `#569CD6`）。

## 5. 已知限制（如实记录）

- **权限审批面板（Permission Request）**：与 Ask 共用同一条「控制面待决请求 → 弹窗」
  管线（`requestKind=approval_required` 分支），映射与渲染由单元测试/验收测试覆盖
  （`test_permission_request_from_pending_mapping`、
  `test_permission_dialog_rendering_contract`）。但当前内置工具声明里没有任何
  `requires_approval=true` 且非 host-mediated 的工具，因此**真实会话中暂时没有可触发
  该分支的工具**，本轮无法产出 live 截图。
- **Plan 面板带数据**：`plan_control` 的 `create` 要求模型声明与注入一致的
  `session_id`，脚本化 stub 无法预先得知运行时会话 id，故 live 截图只有空状态；
  面板的步骤渲染由 `test_plan_mode_toggle_and_rendering_contract` 覆盖。
- **graph resume**：`turn:ask-suspend-stream ... bound_run=None` 说明 TUI 走的轻量
  turn 流不存在图运行绑定，回答后不会 `graph_resume_ask`，而是消费掉待决请求并把
  答案写回工具结果行。需要真正的图运行续行时，TUI 需改用
  `prepare_start_graph_run_stream` 路径（后续波次）。

## 6. 上游 400 的独立缺陷（非 TUI 引入）

真实模型（`antigravity/claude-opus-4-6-thinking`）在工具调用后的 followup 请求被上游拒绝：

```
400 Bad Request -> messages.1.content.0.tool_use.id: Field required
（provider 返回错误状态：400，scope=Followup，decision=Abort NonRetryable）
```

该缺陷会让**任何**工具 followup 失败，与 TUI 交互无关，属于 provider/transcript
回放层的问题，建议单独立项修复。
