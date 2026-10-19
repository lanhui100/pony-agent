# ADR 0042：TUI Composer 收边与底部指标行（弱化 cwd / token / 缓存 / 上下文）

Status: implemented

## 背景

用户实测反馈 `pony` TUI 输入对话框（composer）视觉冗余（wave-39）：

1. **内边距过大**：色带顶部固定 2 行空白 + 输入区底部固定 1 行呼吸 + 色带上方 1 行空白，单行输入实际占用 6 行终端高度；
2. **底部两段信息竞争**：同一行左侧是情境快捷键提示（`Enter 发送 · Shift+Enter 换行` 等 5 态文案），右侧是状态短语（就绪/完成/失败/写入中…），既冗余又与正文流式活动行重复表达状态；
3. **顶部细线多余**：色带上方另有一条整行 `▁` 分隔线，与 header 下方发丝线构成「双线」噪声；
4. **缺少用量反馈**：`TuiState.total_input_tokens/total_output_tokens` 字段存在但从未被更新（`turn:output_end` 载荷里的 `input_tokens/cache_hit_input_tokens/output_tokens/total_tokens` 被 `TuiTurnSink` 全部丢弃），用户看不到 token、缓存命中与上下文占用。

## 决策

1. **几何收边（R5）**：`input_h = visual_lines.clamp(1,5)`、`bottom_h = input_h + 2`、`input_rect.y = bottom_top + 1`、`chat_h = bottom_top - chat_top`。110×30 空输入时：色带占 3 行（1 行顶部内边距 / 输入行 / 指标行），输入行 `y=28`，指标行 `y=29`。
2. **去除顶部细线（R4）**：删除 composer 色带上方的整行 `▁`；全屏仅保留 header 下方 1 条 `hairline()`（ADR 0038 的发丝线语汇不变，「上下双线」降为单线）。
3. **去除快捷键提示与右下角状态（R6）**：5 态 hint 文案与 `status_word()` 不再进入 composer；状态语义由正文流式活动行（`写入中 12秒` 等）与浮层自身承担。`status_word/compact_status` 函数保留（活动行仍在用）。
4. **对话框下方新增一行弱化指标（R7）**：色带最后一行，全部 `theme::FAINT`（ADR 0038 的弱化层级），左对齐 `content.x + RAIL`、右对齐 `content.x + content.width`：
   - 左：当前工作目录 `workspace_label()`（`$HOME`/`%USERPROFILE%` 前缀折叠为 `~`）；
   - 右：`↑{in} ↓{out} · 缓存 {pct} · 上下文 {pct}`；
   - 数字缩写 `<1000` 原样 / `<1e6` 一位小数 `k` / 其余 `M`；空间不足优先完整保留右簇，左路径 `ellipsize_width` 截断，绝不写出内容列。
5. **指标口径（TUI 显示面，独立于 ADR 0035 桌面端口径）**：
   - `in/out` := **跨已完成轮次的累计值**（计费口径），不是「当前上下文输入」；
   - `缓存` := `累计 cache_hit_input_tokens / 累计 input_tokens`，无输入时为 `—`；
   - `上下文` := `最近一轮 total_tokens / 当前模型 context_window_tokens`，窗口未知时为 `—`；窗口来源 = provider 注册表 model capabilities（未命中回退 `1_000_000`）。
   - ADR 0035 的 S1（当前上下文输入 = 最近单次调用输入）约束的是**桌面端**状态栏/圆环；TUI 指标行的累计语义是对不同界面目的的显式选择，两者互不替代、不得互相套用。
6. **usage 事件单源（R8'，review 后加固）**：`UiEvent::StreamUsage` **只**由 `turn:output_end` 发射；`turn:completed` 不再重复发 usage。原因是 completed 与 output_end 携带同一份 `persisted` 值，双发会在「旧轮 completed 迟到 + 新轮失败（无 usage）」时把旧轮 usage 二次计入累计。`apply_turn_usage` 按轮幂等覆盖（先扣旧值再加新值），`begin_turn_usage` 在发送新消息前清空本轮 slot。

## 备选方案与取舍

- **保留底部 hint、仅收窄内边距**：不满足用户「去除快捷键提醒」的明确要求；且 hint 与正文活动行语义重复。
- **指标放到 header 右侧**：header 已有 model/思考强度，加入 4 组数字会破坏单行层级；且用户明确要求「对话框下方一行小字」。
- **给 `StreamUsage` 增加 turn_id 并在主循环按当前 turn 过滤**：能消除双计，但需改动冻结契约面与 UI 主循环锁交互；改为「usage 单源（仅 output_end）」更小闭包且根因消除。
- **指标只显示本轮而非累计**：上下文占用与缓存命中需要跨轮累计才有解释力；本轮口径已由正文活动行承担。
- **`999_950..=999_999` 显示 `1000k`**：接受 `1M`（避免破格写法），A7 数值契约不受影响。

## 后果

- `crates/pony-agent-core/src/bin/tui.rs`：composer 几何、指标行、usage 接线三处变更；新增契约面 `UiEvent::StreamUsage`、`TuiState.{total_cache_hit_tokens,last_context_tokens,context_window_tokens}`、`begin_turn_usage/apply_turn_usage/resolve_context_window_tokens/stream_usage_event`。
- 既有测试适配：`tests/tui_dialog_wave39.rs`（A1-A10 渲染契约）、`tests/tui_usage_wave39_delta.rs`（A11 usage 单源）；`tools/tui-audit/checks.py` 的 separator/footer/composer 行边界同步；`run_audit.sh` 的等待信号改为「指标行非零 token」。
- 已知限制（不在本 ADR 处理）：`char_width` 缺 emoji 区（0x1F680–0x1F6FF 等）可能使含 emoji 的 cwd 少算 1 列而裁掉右簇尾部 1 列（不越界）；E2E 中 4 条 `/thinking` 命令面漂移失败为既有基线问题。

## 关联

- 承接 ADR 0038（TUI 设计系统：发丝线与弱化层级），本 ADR 将其「上下双发丝线」收敛为 header 单线。
- 与 ADR 0035（桌面端 token 指标口径）为不同显示面，口径显式分离。
- 契约与证据：`.dev-team/contract-matrix-tui-dialog-wave39.md`、`.dev-team/evidence/tui-polish/after/`。

## v3 调整（用户实测反馈，同 ADR 内修订）

用户对 v1 效果提出三点：对话框仍偏矮、底部指标行与对话框贴得太近且存在感偏强、无用量时显示 `↑0 ↓0 · 缓存 — · 上下文 —` 属噪声。

1. **对话框增高**：`input_h = (visual_lines + 1).clamp(2, 6)`；单行输入占 2 行（正文 + 1 行呼吸），色带 = 1 行顶部内边距 + input_h。
2. **指标行脱离色带**：色带下方先 1 行 CANVAS 间距行（不画字符），再 1 行 CANVAS 背景的指标行；`bottom_h = input_h + 3`。
3. **更弱的层级**：新增 `theme::GHOST = Rgb(104,98,90)`（介于 `HAIRLINE` 与 `FAINT` 之间），指标行整行使用。终端无法改变字号，「更小更弱」以更低对比度实现；不使用 `Modifier::DIM`（终端兼容性差，且项目既有 0 使用）。
4. **无值不显示**：右侧指标改为分段拼装，`↑in ↓out` 仅当有非零用量、`缓存 x%` 仅当输入 token>0、`上下文 y%` 仅当窗口已知；全无值时右簇为空，只保留左侧 workspace。彻底消除 `—` 占位符与 `↑0 ↓0`。
5. 测试相应迁移：A5/A6/A8 重写，新增 A12（GHOST + CANVAS + 间距行逐格）与 A13（分段隐藏组合、无孤立分隔符）；E2E `checks.py` 空态改为 workspace-only，落定场景仍要求四段完整。
