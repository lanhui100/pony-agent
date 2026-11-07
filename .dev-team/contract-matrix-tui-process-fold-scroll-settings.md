# 契约矩阵 · TUI 右侧滚动条 + 思考折叠 + `/settings` 面板（Agent 过程隐藏）

> 真值源（Lead 冻结）。改判必须在本文件**原地改写**并登记单行变更流水。
> 交付文件：`crates/pony-agent-core/src/bin/tui.rs`（bin `pony`）、
> 新增 `crates/pony-agent-core/src/agent/tui_preferences.rs`（+ `agent/mod.rs` 注册）。
> 分级：**A 级**（单模块 UI + 纯本地偏好文件，无并发/网络/LLM 调用新增）。
> 验收测试（唯一新增）：`crates/pony-agent-core/tests/tui_process_fold_acceptance.rs`（红相冻结后只读）。

## 0. 用户诉求 → 契约映射

1. 「tui 中增加右侧边滚动条」→ C1/C2/C3。
2. 「思考折叠」→ C4/C5。
3. 「增加 `/settings` 命令弹出面板」→ C6/C7/C8。
4. 「增加折叠 agent 过程的能力：将轮次间的工具调用和思考全部隐藏，仅显示状态消息；
   状态消息中的工具调用显示当前的工具信息；可开关，默认为隐藏」→ C9/C10/C11/C12。

## 1. 契约条目（每条 = 一条可机械断言）

| ID | 断言（行为真值） | 断言载体 |
| :-- | :-- | :-- |
| **C1** | 滚动条几何纯函数 `scrollbar_thumb(total, visible, offset) -> Option<(u16, u16)>`：`visible == 0` 或 `total <= visible` ⇒ `None`；否则 `thumb_len = max(1, round(visible * visible / total))`，`start = round(offset * (visible - thumb_len) / max(1, total - visible))`，`start + thumb_len <= visible`（饱和，不越界）。 | A1（纯函数表驱动） |
| **C2** | 渲染位置：终端**最右列**（`area.x + area.width - 1`），纵向仅覆盖消息区 `chat_rect.y .. chat_rect.y + chat_rect.height`；不侵占正文列（正文列宽不受影响）。轨道字形 `│`（`theme::GHOST`），滑块字形 `█`（`theme::ACCENT_DIM`）。 | A2（TestBackend 逐格断言） |
| **C3** | 不溢出时不绘制：`scroll_total <= visible` 时该列全为背景（无 `│`/`█` 字形）。空态（`messages.is_empty()`）同样不绘制。 | A3 |
| **C4** | 思考折叠：`thinking_collapsed == true` 时 `MessageRole::Thinking` 块只产出 **1 行**摘要，文本恒为 `format!("✻ 思考 · {n} 行 · 已折叠", n)`，其中 `n = message.content.lines().count()`（空内容 ⇒ `0`），首前缀 `✻ ` 沿用 `theme::ACCENT_DIM`，后半段 `theme::FAINT`。折叠态该块**尾部仍保留 1 行呼吸**（C6/C7 既有契约不变）。 | A4/A5（`message_lines_for_with_flags` + 渲染） |
| **C5** | 展开态（`thinking_collapsed == false`）保持既有逐行渲染（`✻` 前缀 + FAINT/ITALIC 正文 + RAIL 续行），与 HEAD 行为逐字一致。 | A5 + 既有 `tui_thinking_bottom_gap` 回归 |
| **C6** | `/settings` 进入 `InteractionMode::Settings`；命令表 `AVAILABLE_COMMANDS` 含 `("/settings", "设置 TUI 显示与交互偏好")`；`is_panel_command("/settings")` 为真（流式期可打开，不排队、不拒绝）。 | A6（命令解析）+ 代码审阅 |
| **C7** | 面板行：`settings_rows()` 返回按固定顺序 `[("hide_process", …), ("thinking_collapsed", …)]`；每行含 `[开]/[关]` 状态标记 + 名称 + 说明；↑↓ 选择（首尾不环绕）、`Space`/`Enter` 切换选中项、`Esc` 关闭并回到 `Normal`。 | A7/A8（状态机 + TestBackend 面板断言） |
| **C8** | 切换即时生效且**不写盘**在状态机层：`set_hide_process` / `set_thinking_collapsed` 只改内存并 `invalidate_render_cache()`；持久化由事件循环在切换后调用 `TuiPreferencesStore::save`。 | A8 + A9（store 单元） |
| **C9** | 隐藏过程：`hide_process == true` 时 `MessageRole::{Thinking, ToolCall, ToolResult}` 的渲染块**行数为 0**；`User` / `Assistant` / `System` 不受影响。判据纯函数 `should_hide_message(role, hide_process)`。 | A10/A11 |
| **C10** | 隐藏态流式状态行必须显示当前工具信息：`is_streaming && hide_process && runtime_mode == ExecutingTool(_)` 时，消息区末尾状态行**照常渲染**（HEAD 在该态整行不渲染），文案形如 `执行 · <工具名>[ · <摘要>]`，其中工具信息取自最近一条 `ToolCall` 消息内容（`truncate` 到可用宽度）；`stopping` 分支优先级不变。 | A12（TestBackend 断言含 `执行 · ` 与工具名，且不含工具行字形 `◦/✓/✕`） |
| **C11** | 默认为隐藏：`TuiPreferences::default().hide_process == true`、`thinking_collapsed == true`；`run_tui` 启动时 `state.apply_tui_preferences(&store.load())` ⇒ 用户侧默认隐藏 + 折叠。**修订 R1（2026-11-07，Lead）**：`TuiState::default()` 保持**旧行为**（`hide_process = false`、`thinking_collapsed = false`，与 `model_preferences` 同一约定「Default 不做 IO、保持可复现基线」），理由：已冻结的 `tui_thinking_bottom_gap` A6/A11 直接以 `TuiState::default()` 断言思考块可见，若把产品默认塞进 `Default` 会破坏只读契约测试；产品默认由偏好加载点体现。 | A13 + A9 |
| **C12** | 快捷开关：`Ctrl+O`（`InteractionMode::Normal`）切换 `hide_process` 并落盘，状态栏给出一次性提示（`set_status_notice`）。`/settings` 与快捷键必须读写**同一**字段。 | A14 |

## 2. 契约冻结符号（Lead 唯一写主；Test Agent 只读引用）

### 2.1 新增核心模块 `agent::tui_preferences`

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TuiPreferences {
    #[serde(default = "default_true")] pub hide_process: bool,
    #[serde(default = "default_true")] pub thinking_collapsed: bool,
}
impl Default for TuiPreferences { /* hide_process: true, thinking_collapsed: true */ }

pub struct TuiPreferencesStore { /* <config_dir>/pony-agent/tui-preferences.json */ }
impl TuiPreferencesStore {
    pub fn new() -> Self;
    pub fn with_path(path: impl Into<PathBuf>) -> Self;
    pub fn load(&self) -> TuiPreferences;                 // 缺失/损坏 ⇒ 默认值
    pub fn save(&self, prefs: &TuiPreferences) -> Result<(), String>; // 原子写
}
```

### 2.2 `tui.rs` 新增/变更符号

```rust
// 纯函数（可单测）
pub fn scrollbar_thumb(total: usize, visible: usize, offset: usize) -> Option<(u16, u16)>;
pub fn should_hide_message(role: &MessageRole, hide_process: bool) -> bool;
pub fn thinking_fold_label(content: &str) -> String;                 // "✻ 思考 · {n} 行 · 已折叠"
pub fn message_lines_for_with_flags(message: &ChatMessage, width: usize, hide_process: bool, thinking_collapsed: bool) -> Vec<Line<'static>>;

// InteractionMode 新增变体
InteractionMode::Settings

// TuiState 新增字段 / 方法
pub hide_process: bool;              // 默认 true
pub thinking_collapsed: bool;        // 默认 true
pub settings_selected_index: usize;  // 默认 0
pub fn apply_tui_preferences(&mut self, prefs: &TuiPreferences);
pub fn set_hide_process(&mut self, value: bool);
pub fn set_thinking_collapsed(&mut self, value: bool);
pub fn toggle_hide_process(&mut self) -> bool;
pub fn open_settings(&mut self);
pub fn close_settings(&mut self);
pub fn settings_rows(&self) -> Vec<(&'static str, &'static str, bool, &'static str)>; // (key, label, enabled, desc)
pub fn move_settings_selection(&mut self, delta: i32);
pub fn toggle_selected_setting(&mut self) -> Option<(&'static str, bool)>;

// 渲染
fn render_settings_panel(f: &mut Frame, state: &TuiState);
fn render_scrollbar(f: &mut Frame, area: Rect, state: &TuiState);
```

`MessageRenderCtx` 追加 `hide_process: bool` / `thinking_collapsed: bool` 两个字段；
公共包装 `message_lines_for` 保持**旧行为**（两者均为 `false`），供既有虚拟化测试做参考路径。

## 3. 持久化与观测性

- 偏好文件与 `model-preferences.json` 同目录、**独立成文件**（不污染 `providers.json` / 桌面端 `settings.json` 契约）；
  原子写：`*.json.tmp` → rename；缺失或解析失败一律回落默认值（向前兼容）。
- 观测性动线：**不适用**（纯终端渲染 + 本地偏好文件；无对外接口、网络 IO、后台任务、LLM 调用新增）。
  依据 `docs/observability.yaml` 既有结构性 N-A 段落（TUI 交付形态随进程启停、无常驻监听面），
  本轮仅在 `docs/observability.yaml` 追加一行归属登记，不新增收口点、不伪造 `/metrics`。

## 4. 变更流水

- 2026-11-07：冻结首版（C1–C12）。
- 2026-11-07：T-C 文档与观测性归属登记完成。

## 5. 交付状态（T-C 追加；不改写 C1–C12 断言）

- 文档：`docs/decisions/0079-tui-process-fold-scroll-settings.md`（Status: implemented，2026-11-07）；
  索引 `docs/decisions/README.md` 已追加 0079 行。
- 观测性：`docs/observability.yaml` 追加「wave-tui-process-fold 归属登记」——
  四维归属不变（tracing/issues 适用、metrics/logs 结构性 N-A），不新增收口点、不伪造 `/metrics`。
- 任务快照（截至本行写入）：T-A 完成（`agent::tui_preferences`）；T-D 已落地并通过
  `cargo check --bin pony`；T-B（A1–A14 红相冻结）进行中；T-E（验收复跑）/ T-F（独立评审）待执行。
