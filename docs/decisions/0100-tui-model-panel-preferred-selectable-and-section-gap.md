# 0100 TUI 模型面板：优选行参与统一导航与分区间 1 行间距

Status: implemented

## 背景

TUI `/models` 与提供商联动打开的模型面板把「优选」栏前置于「全部模型」之上，但旧实现里优选行仅展示（`list_row(false, …)`），键盘导航索引空间只覆盖 `model_list`：↑↓ 永远进不了优选栏，回车也无法确认优选行——用户看到优选模型却选不中。同时优选栏末行与「全部模型」caption 紧贴，缺少视觉分组间距。

## 候选方案

- **A 统一导航索引空间（采纳）**：索引空间 = 优选栏行数 + 全部模型行数，索引与渲染行一一对应；高亮、↵ 确认、Ctrl+D/Ctrl+F 共用 `model_panel_item_at` 映射，`model_panel_index_of` 负责按 id 反查。同一模型允许在两栏各出现一次。优点：所见即所选，映射单点闭合；缺点：Ctrl+F 切换优选会平移索引空间，需要重锚（已用 id 重锚解决）。
- **B 保留旧索引空间，优选行随全列表同 id 高亮**：改动最小，但优选行仍不可独立导航与确认，没有修复用户报告的问题，落选。
- **C 去重导航（优选模型从全部模型栏移除）**：导航无重复行，但「全部模型」栏不再完整，Ctrl+F / `r` 刷新后的窗口锚点与行映射需要额外状态维护，高内聚程度反而下降，落选。

## 决策

模型面板采用统一导航索引空间：`selected_model_index` 语义为「优选栏前置 + 全部模型」的合成行号；所有消费点（`open_model_selector` 初始锚定、`next_model` 边界、`select_current_model`、`apply_model_panel_shortcut`、`ModelsDiscovered` 刷新重锚、渲染高亮）一律经 `model_panel_len` / `model_panel_item_at` / `model_panel_index_of` 三个辅助函数闭合，禁止再直接以 `model_list` 下标解释面板选中。三函数落在 `crates/pony-agent-core/src/bin/tui/model_panel.rs`（沿用 ADR 0089 拆分口径，避免 tui.rs 规模棘轮恶化）。Ctrl+F 切换优选与远程刷新后按模型 id 重锚选中，保证索引空间变化时选中身份不漂移。优选栏与「全部模型」caption 之间渲染 1 行页面底色空行（面板高度预算同步 +1）。

## 影响

- `selected_model_index` 不再是 `model_list` 下标；新增消费者必须走上述辅助函数（靠 review + 验收测试守卫）。
- 机械门禁：`crates/pony-agent-core/tests/tui_six_fixes_acceptance.rs` 四项新增验收（优选行可导航可确认、高亮随选中、分区间 1 行空行、Ctrl+F 重锚不漂移）+ 既有 `tui_acceptance::test_model_selector_state_logic` 边界回归。
- 面板标题计数 `{selected+1}/{total}` 改为统一空间口径（含优选重复行）。