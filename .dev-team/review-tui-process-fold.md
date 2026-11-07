# 独立评审 · TUI 右侧滚动条 + 思考折叠 + `/settings` 面板 + Agent 过程默认隐藏

评审人：docs-reviewer（T-F，独立于实现方）
日期：2026-11-07
判据：`.dev-team/contract-matrix-tui-process-fold-scroll-settings.md` C1–C12（含修订 R1）
评审方式：只读 `git diff` + 代码通读 + 纯函数/状态机逐条对照 + 复跑 `cargo check` 与验收测试（未改任何代码）

## 0. 评审快照（行号以该哈希为准，文件仍在被实现方改动时行号会漂移）

| 文件 | sha256（前 12） |
| :-- | :-- |
| `crates/pony-agent-core/src/bin/tui.rs` | `397562e595af` |
| `crates/pony-agent-core/src/agent/tui_preferences.rs` | `ecd8c7351f73` |
| `crates/pony-agent-core/tests/tui_process_fold_acceptance.rs` | `be111f6cd711` |

**排除范围**：`tui.rs` 同时承载他人前序未提交改动（ADR 0075 终端还原 / Ask 面板换行等）。
本评审只针对本轮四项能力（滚动条 / 思考折叠 / `/settings` / 过程隐藏）及其触点，前序改动不作判据。

## 1. 机器证据

| 命令 | 结果 |
| :-- | :-- |
| `cargo check --bin pony --message-format short` | exit 0；`tui.rs` / `tui_preferences.rs` **零告警**；lib 侧 14 条 `agent/team_history.rs` unused-variable 告警为存量（该文件本轮未改） |
| `cargo test --test tui_process_fold_acceptance` | `41 passed; 0 failed`（含 A1–A14 全绿） |

## 2. 逐条判定

| 契约 | 实现位置 | 判定 | 理由 |
| :-- | :-- | :-- | :-- |
| **C1** | `tui.rs:6994` `scrollbar_thumb` | **PASS** | `visible==0 \|\| total<=visible ⇒ None`；`thumb_len = (vis² + total/2)/total`（四舍五入）且 `clamp(1, vis)`；`start = (offset.min(denom)*track + denom/2)/denom`，再 `.min(track)` ⇒ `start+thumb_len<=visible`。与契约公式逐项一致；A1 通过。 |
| **C2** | `tui.rs:7166` `render_scrollbar`，调用点 `tui.rs:7608` | **PASS**（见 ADVISORY-1） | `x = screen.x+width-1`；纵向仅 `0..chat_rect.height`，写屏前逐行判 `y/x` 上界（不越界）。轨道 `│`/`GHOST`、滑块 `█`/`ACCENT_DIM` 与契约一致。正文列不受影响：`chat_rect` 宽 = `min(width-6,120)`（`GUTTER=3`），右缘 ≤ `width-1`，A2 断言 `chat.x+chat.width <= width-1` 成立。 |
| **C3** | `tui.rs:7167` 早退 | **PASS** | `messages.is_empty() \|\| max_scroll_offset == 0` 直接 return；A3 断言未溢出态与空态该列全为 `' '`。 |
| **C4** | `tui.rs:6986` `thinking_fold_label`；折叠渲染 `tui.rs:6835` 起 | **PASS** | 文案恒为 `✻ 思考 · {content.lines().count()} 行 · 已折叠`（空内容 ⇒ `0`）；`✻ ` 用 `ACCENT_DIM`、后半段 `faint()`；折叠块尾 `lines.push(blank())` 保留 1 行呼吸；A4/A5 通过。 |
| **C5** | 折叠分支之后的原展开分支 | **PASS** | 展开分支逐行 `✻` + `FAINT`+`ITALIC` + RAIL 续行，代码未被改写（仅在 Thinking 分支前插入折叠早退）；既有 `tui_thinking_bottom_gap` 仍绿。 |
| **C6** | `tui.rs:1037` `is_panel_command`；`tui.rs:1643` 命令表；`tui.rs:11374` 分发；`tui.rs:8078` 渲染 | **PASS** | `/settings` 进 `PANEL_COMMANDS`（trim + 大小写不敏感）、进 `AVAILABLE_COMMANDS`（描述逐字一致）、`open_settings()` 进 `InteractionMode::Settings`。流式期：`streaming_key_disposition` 只拦 Enter 语义，`/settings` 经 `Forward` 落到 Enter 分发 ⇒ 面板打开，不排队、不拒绝。`InteractionMode::Settings` 也进 `is_modal_mode`（`tui.rs:1127`），面板态按键不再漏进 Composer。 |
| **C7** | `tui.rs:3467` `settings_rows`；`tui.rs:3516` `handle_settings_key`；`tui.rs:7200` `render_settings_panel`；`tui.rs:10591` 事件循环分派 | **PASS**（见 ADVISORY-5） | 行序固定 `hide_process → thinking_collapsed`；每行 `[开]/[关]` + 名称 + 说明；`↑↓` 用 `clamp(0,last)`（首尾不环绕）；`Space`/`Enter` 切换、`Esc` 回 `Normal`；面板态吸收全部按键并 `continue`。额外支持 `j/k`、`q`（additive）。 |
| **C8** | `tui.rs:3430/3438` `set_*`；`tui.rs:3492` `toggle_selected_setting`；`tui.rs:6673` `persist_tui_preferences`；写盘点 `tui.rs:10592`、`tui.rs:11248` | **PASS** | 状态机只改内存 + `invalidate_render_cache()`，无任何 IO；落盘唯一写入点由事件循环在 `handle_settings_key` 返回 `Some` / `Ctrl+O` 之后调用；A8/A9 通过。 |
| **C9** | `tui.rs:6977` `should_hide_message`；早退点 `tui.rs:6717`（`message_lines_with_ctx` 顶部） | **PASS** | 仅渲染层过滤：`messages`、持久化历史、滚动/选区行映射（按消息分块）全部保留；`User/Assistant/System` 不受影响；A10/A11 通过。 |
| **C10** | `tui.rs:6649` `hidden_tool_status_text`；状态行条件 `tui.rs:7445-7478` | **BLOCKING**（主路径 PASS，`stopping` 优先级被破坏） | 主断言成立：`is_streaming && hide_process && ExecutingTool(_)` 恢复状态行，文案 `执行 · <最近 ToolCall 内容>`（宽度截断），A12 通过且屏幕上无 `◦/✓/✕`。**缺陷**：`hidden_tool_status_text` 只判 `runtime_mode == ExecutingTool`，未判 `stopping_turn_id`；`tui.rs:7468` 的 override 会把 `status_word` 已裁定的「正在停止…」替换成「执行 · …」。契约「`stopping` 分支优先级不变」不成立。 |
| **C11** | `tui.rs:2774-2775`（`Default` 基线）；`tui.rs:3423` `apply_tui_preferences`；`tui.rs:9101` 启动加载；`tui_preferences.rs:32-33` | **PASS** | 按 R1 口径：`TuiState::default()` 仍是「可见 + 展开」且不做 IO（保冻结断言）；产品默认「隐藏 + 折叠」由 `TuiPreferences::default()`（两者 true）在 `run_tui` 启动时经 `apply_tui_preferences` 落地并整表失效缓存；A13 通过。 |
| **C12** | `tui.rs:11247` `KeyCode::Char('o')`（CONTROL 块，位于各 mode 分支之后、通用 `Char` 插入之前）；`tui.rs:3446` `toggle_hide_process`；`tui.rs:7468` 同字段 | **PASS**（见 ADVISORY-7） | Normal 下 `Ctrl+O` 切换 + `persist_tui_preferences` + `set_status_notice` 一次性提示；因在通用 `Char(c) => insert_char(c)` 之前消费，不会把 `o` 插进输入框；与 `/settings` 读写同一字段（`settings_rows` 直接读字段，无影子状态）；A14 覆盖同字段与缓存即时生效。 |

## 3. BLOCKING（1 项）

### B1 · C10「`stopping` 分支优先级不变」被隐藏态工具文案覆盖

- **证据位置**：`crates/pony-agent-core/src/bin/tui.rs:7465-7478`（状态行文字裁定），`tui.rs:6649`（`hidden_tool_status_text`）
- **触发条件**：`is_streaming && hide_process && runtime_mode == ExecutingTool(_) && stopping_turn_id.is_some()`（工具执行中双击 Esc 请求停止）
- **现象**：`status_word` 已按 wave-60 §C9 产出「正在停止…」（或其停止提示），随后被 `hidden_tool_status_text` 返回的 `执行 · <工具>` 无条件覆盖 ⇒ 用户提交停止请求后看不到「正在停止…」，与契约 C10 明文冲突。
- **可执行修复（二选一，推荐①）**：

① 在 `tui.rs:6649` `hidden_tool_status_text` 函数体**首行**（`let tool_name = ...` 之前）加判据，使判据收敛在纯函数内（A12 可机械断言）：

```rust
// 契约 C10：`stopping` 分支优先级不变 —— 停止请求受理后状态行必须呈现「正在停止…」，
// 不得被隐藏态的工具信息覆盖（wave-60 §C9 的可观测性要求）。
if state.stopping_turn_id.is_some() {
    return None;
}
```

② 或把 `tui.rs:7468` 的 override 条件改为 `if state.hide_process && state.stopping_turn_id.is_none()`（等价，但判据分散在渲染路径）。

- **修复后需注意**：`None` 分支会回落到 `(word, style)`，即既有 `status_word` 口径，无需其它改动。
- **测试建议（交 Lead / T-B 决定，测试文件只读不由我改）**：A12 追加一个 `stopping_turn_id = Some(..)` + `ExecutingTool` 的断言分支，锁死「隐藏态下停止提示不被工具信息覆盖」。

## 4. ADVISORY（不改代码，仅登记）

1. **滚动条与正文距离**：`GUTTER=3` + 内容限宽 120，超宽终端（如 200 列）下滚动条贴屏幕最右列、正文居中，两者可能相距数十列。这是契约 C2 的显式选择（最右列 = 不侵占正文），不改；若用户反馈「找不到滚动条」，需回到 C2 改判而非就地挪动。
2. **隐藏态的复制/选区面**：`rendered_lines_text` 与渲染缓存同源，`hide_process` 打开时 Ctrl+C/选区复制拿不到隐藏内容（数据未删，展开后可复制）。已在 ADR 0079「已知残余」登记。
3. **状态行工具信息可能跨轮陈旧**：`hidden_tool_status_text` 取「最近一条 `ToolCall`」，新轮 `ExecutingTool` 已置但工具消息尚未落时，会短暂显示上一轮工具。构造上窗口极短，登记为待观测风险。
4. **隐藏态状态行呼吸**：`need_blank` 由「最后一条缓存块最后一行非空」判定；隐藏态末尾块可能为空 ⇒ 状态行有时紧贴上一可见行（少 1 行呼吸）。不影响 C10 断言，属观感项。
5. **重复注释（新增）**：`tui.rs` 中 `wave-60 §C9：stopping 是用户此刻唯一关心的进行中态…` 连续出现两次（HEAD 仅 1 次），本轮新增时带出重复；建议删除其一（纯注释，无行为影响）。
6. **`/settings` 面板附加按键未登记**：`j/k` 导航、`q` 关闭为契约 C7 之外的 additive 行为，ADR 0079 只写了 `↑↓/Space/Enter/Esc`；文档口径可补一句。
7. **测试缺口（不阻塞）**：C6「流式期可打开」与 C12「`Ctrl+O` 按键分支」目前只有状态机级断言（A6 判 `is_panel_command` + `open_settings`，A14 判 `toggle_hide_process`），没有驱动真实按键分派的用例；建议 T-E 补一条事件级用例或 PTY 取证。
8. **面板态 Esc 语义**：Settings 按键分支位于全局「流式态 Esc = 停止本轮」之前 ⇒ 面板打开时 Esc 只关面板，不触发停止（与 ADR 0071 面板优先级一致，但值得在 ADR 里写明）。
9. **滚动条在模态期不绘制**：仅在 `Normal | PlanMode` 绘制，Ask 覆盖层（`AskUserQuestion`）打开时滚动条消失。契约未要求，登记为已知行为差异。
10. **`-D warnings` 存量风险**：本轮两文件零告警；但 lib 侧 `agent/team_history.rs` 已有 14 条 unused-variable 告警，若 CI 用 `-D warnings` 会连带失败——属存量问题，建议另开任务，不在本轮夹带修复。

## 5. 结论

- C1–C9、C11、C12：**PASS**。
- C10：**BLOCKING × 1**（B1，修复量 ~3 行，判据收敛在 `hidden_tool_status_text` 即可）。
- 修掉 B1 后，本波可进入 T-E 验收入口；无需改动契约 C1–C12。
