# 契约矩阵 —— TUI 输入对话框(Composer) 收边 + 底部指标行 (wave-39)

## 0. 事实基线（Explorer 证据，@566e465）
- Composer 不是 Block：无边框/无标题/无 `Padding`，是 `SURFACE` 填充色带 + 手算 Rect。`crates/pony-agent-core/src/bin/tui.rs:2390-2402`；`grep "Padding"` = 0。
- 顶部细线 = 色带上一行的整行 `"▁"`（U+2581），`tui.rs:2403-2418`；header 发丝线同字形 `hairline()` `tui.rs:1473-1478,2118-2119`。
- 当前尺寸：`input_h=(visual_lines+1).clamp(2,6)`，`bottom_h=input_h+3`，`input_rect.y=bottom_top+2`（顶部 2 行空白内边距），状态行占色带最后一行（=终端最后一行）。`tui.rs:2122-2127,2420-2447`。
- 底部快捷键提示（左）+ 右下角状态短语（右）同处该状态行。`tui.rs:2428-2478`；提示串 `2431-2440`；`status_word()` `2010-2031`。
- `chat_h = bottom_top - chat_top - 1`（色带上方额外 1 行空白）。`tui.rs:2129-2136`。
- 指标数据源：`TurnStreamEvent.input_tokens/cache_hit_input_tokens/output_tokens/total_tokens` 已由 `turn:output_end` 载荷携带（`src/agent/runtime/turn_stream.rs:2396-2412` 等）；`TuiTurnSink` 当前全部丢弃（`tui.rs:1199-1205`）。
- `TuiState.total_input_tokens/total_output_tokens` 存在但从未被更新（`tui.rs:250-251,297-298`）；无 cache/context 字段。
- 上下文档位：注册表 `ProviderModelConfig.capabilities.context_window_tokens: Option<u32>`（`src/agent/config.rs:128-146,77-92`）；TUI 合成 provider 硬编码 `Some(1_000_000)`（`tui.rs:1102`，与 `config.rs:11 DEFAULT_CONTEXT_WINDOW_TOKENS` 一致）。
- 弱化样式：`faint()` = `theme::FAINT` Rgb(124,118,110)（`tui.rs:1373,1409-1411`）。禁止 `Modifier::DIM`/`DarkGray`（项目 0 使用）。

## 1. 新增契约面（Protocol Agent 唯一编写）
### R1 `UiEvent::StreamUsage`
```rust
StreamUsage { input_tokens: u64, output_tokens: u64, cache_hit_input_tokens: u64, total_tokens: u64 }
```
### R2 TuiState 字段（均 pub，Default=0）
- `total_input_tokens: u64` / `total_output_tokens: u64`（已存在，语义改为「累计已提交轮次」）
- `total_cache_hit_tokens: u64`（新）
- `last_context_tokens: u64`（最近一轮 total_tokens；新）
- `context_window_tokens: u32`（0 = 未知；新）
- 内部去重字段 `turn_usage: Option<(u64,u64,u64,u64)>`（私有）
### R3 方法签名
- `TuiState::begin_turn_usage(&mut self)`：清空本轮回执（发送新消息前调用）。
- `TuiState::apply_turn_usage(&mut self, input: u64, output: u64, cache_hit: u64, total: u64)`：**幂等覆盖**当前轮增量 —— 若已有本轮回执则先扣除旧值再加入新值，`last_context_tokens=total`；跨轮为累加。
- `resolve_context_window_tokens(state: &TuiState) -> u32`：注册表命中当前 provider 且模型 id/model 命中 `state.model` → `capabilities.context_window_tokens`；否则回退 `1_000_000`。

## 2. 渲染契约（Executor 实现）
### R4 去除顶部细线
删除 `tui.rs:2403-2418` 的 `"▁"` 色带分隔行；画布上色带上一行保持空白（不画任何字符）。
### R5 内边距收小（几何，硬数字）
- `input_h = visual_lines.clamp(1, 5)`（去掉输入底部的固定呼吸行；单行输入只占 1 行）
- `bottom_h = input_h + 2`（1 行顶部内边距 + input_h + 1 行指标行）
- `input_rect.y = bottom_top + 1`
- `chat_h = bottom_top - chat_top`（去掉色带上方额外 1 行空白）
- 110×30 空输入基准：`bottom_top=27`，色带 27/28/29 = 空行 / 输入 / 指标。
### R6 去除底部快捷键提示与右下角状态
- 不再渲染 `2428-2478` 的 hint/status 组合；`hint` 五条串全部消失。「Esc 中断」等语义无需迁移（浮层/正文已承载）。
- `status_word()` 仍被正文流式活动行使用（`tui.rs:2339`），**保留**；`compact_status` 同样保留。
### R7 对话框下方指标行（唯一新增视觉元素）
- 位置：色带最后一行（`y = input_rect.y + input_h`），整宽 `SURFACE`，全部 `faint()`。
- 左对齐（起点 `content.x + RAIL`，与输入正文同列）：当前所在文件夹 `workspace_label(&state.workspace)` = `workspace.display()`，若以 `$HOME` 开头则替换为 `~`。
- 右对齐（终点 `content.x + content.width`）：`↑{in} ↓{out} · 缓存 {cache%} · 上下文 {ctx%}`
  - `in = total_input_tokens`，`out = total_output_tokens`
  - 数字格式：`<1000` 原样；`<1_000_000` → 一位小数 `k`（`12.3k`，`.0` 省略）；否则 `M` 一位小数。
  - `cache% = round(total_cache_hit_tokens / total_input_tokens * 100)`，`input==0` → `—`
  - `ctx%  = round(last_context_tokens / context_window_tokens * 100)`，`clamp(0,100)`，`window==0` → `—`
- 空间不足：优先保证右侧指标簇完整，左侧路径用 `ellipsize_width` 截断；`RAIL + 1 + right_w > width` 时只渲染右簇并截断到 `width`；绝不写到内容列外（沟槽列必须保持空白）。
### R8 数据接线
- `TuiTurnSink::emit`：`"turn:output_end" | "turn:completed"` 分支在发送 `StreamCompleted` 前，若载荷任一带 token 字段则发 `UiEvent::StreamUsage`。
- 主循环 `StreamUsage`：调用 `apply_turn_usage`；发送新消息前调用 `begin_turn_usage` 并 `state.context_window_tokens = resolve_context_window_tokens(&state)`。
- `UiEvent` 新增分支必须被 match 覆盖（无 `_ =>` 漏网）。

## 3. 验收契约（Test Agent 冻结，红相先行）
新建 `crates/pony-agent-core/tests/tui_dialog_wave39.rs`（`#[path="../src/bin/tui.rs"] mod tui;`，TestBackend 渲染真断言）：
- A1 色带上方无 `"▁"` 行；全屏 `"▁"` 行恰好 = 1（header rule）。
- A2 正常态全屏不含 `"Enter 发送"`/`"Esc 中断"`/`"↑↓ 选择"`/`"Shift+Enter"`。
- A3 最后一行不含 `就绪/完成/失败/待确认/写入中/思考中/工具 ·`。
- A4 最后一行左起第 `content.x+2` 列开始为路径标签（构造 workspace 断言出现 basename / `~` 形式）。
- A5 最后一行右端（内容列最后一列）以 `%` 或数字结束，且含 `↑`/`↓`/`缓存`/`上下文`。
- A6 几何：110×30 空输入 → 提示符行 `y=28`，指标行 `y=29`，`y=27` 全空白；`bottom_h=3`。
- A7 数值：注入 in=1200/out=345/cache=600/last_ctx=50000/window=200000 → 行内含 `1.2k`、`345`、`缓存 50%`、`上下文 25%`。
- A8 未知态：in=0 → `缓存 —`；window=0 → `上下文 —`。
- A9 `apply_turn_usage` 幂等：同一轮两次调用不翻倍；两轮累加正确；`last_context_tokens` 跟随。
- A10 退化：宽度 60/80 不 panic、不越沟槽；浮层态（mode!=Normal）不渲染色带。

## 4. Non-Goals
- 不改 header（模型/思考强度留在顶部）、不改模态面板、不改正文/工具行。
- 不改前端 Vue / 桌面端。
- 不修 `tests/tui_render_layout.rs` 既有 5 个陈旧失败（基线已记录：3 passed / 5 failed，与本波次无关）。
- 不新增第三方依赖。

## 5. 契约变更 v2（wave-39-delta，Lead 裁决 2026-10-19）

触发：L3-A 审查非机器观察（PASS 未入 findings，Lead 双人复核后判定为真实可达缺陷）：
`turn:output_end` 与 `turn:completed` 均携带同一 `persisted` token 值，主循环对两者各 apply 一次；
当用户在旧轮 output_end 后、旧轮 completed 之前提交新轮，且新轮失败/取消（无 usage 回执）时，
旧轮 usage 会以「本轮回执」身份被二次计入累计 → 指标永久多算。

### R8'（替代 R8 中 usage 发射口径）
- `StreamUsage` **仅**由 `"turn:output_end"` 发射；`"turn:completed"` 只发 `StreamCompleted`，不再重复发 usage。
- 抽取纯函数（可测）：
  `pub fn stream_usage_event(event_name: &str, payload: &TurnStreamEvent) -> Option<UiEvent>`
  规则：`event_name == "turn:output_end"` 且任一 token 字段为 Some → `Some(StreamUsage{..})`（缺省 0）；否则 `None`。
- `TuiTurnSink::emit` 在 `"turn:completed" | "turn:output_end"` 分支先 `if let Some(ev) = stream_usage_event(name, &payload) { send(ev) }`，再发 `StreamCompleted`。

### 验收 A11（Test Agent 新增独立文件，不改 A1-A10 冻结文件）
- `stream_usage_event("turn:output_end", 全量 payload)` → Some，四个数值一致。
- `stream_usage_event("turn:completed", 同载荷)` → None（双发去重根因）。
- `stream_usage_event("turn:delta"/"turn:failed", payload)` → None。
- 缺失字段（仅 output_tokens=Some(5)）→ Some，其余为 0。

### R7 注释级裁决（不改实现）
- `fmt_token_count` 在 999_950..=999_999 输出 `1M`（而非字面 `1000k`）：Lead 裁决**接受**，避免破格写法；A7 不受影响。
- `char_width` 缺 emoji 区（0x1F680–0x1F6FF 等）属既有宽度表局限（HEAD 同样存在），本波次不修，记录为已知限制。
- E2E 4 条失败（empty.command_desc_column、slash.all_commands_listed/each_command_once/command_desc_column）经 pre-wave-39 二进制 1:1 复现，判为既有基线漂移（`/thinking` 后 8 命令 vs 冻结 7 命令期望），非本波次回归，另开工单。
