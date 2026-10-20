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
