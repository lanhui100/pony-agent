# ADR 0078：TUI header 承载工作区标签与数值指标，底面行回归模型身份

Status: implemented

日期：2026-11-07
契约真值源：`crates/pony-agent-core/tests/tui_render_layout.rs`（`layout_invariants_hold_on_standard_terminal`、
`panel_renders_inside_screen_and_keeps_footer`、`header_workspace_label_is_home_folded_folder_name`）与
`crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs`（`o1_*`）

## 背景

旧布局把**两类语义塞在同一行**：composer 下方最后一行左侧是折叠 cwd 路径、右侧是 token/缓存/上下文指标。
实测暴露三个问题：

1. **一行两义**：路径（身份：我在哪）与指标（状态：花了多少）挤在同一行，终端变窄时互相挤占，
   指标被截断得只剩 `↑1` 这种不可读片段，路径也会被省略成 `…ony-agent`。
2. **路径重复且不紧凑**：cwd 绝对路径在 header 缺失、在底部整行铺开，占用最宝贵的常驻行；
   用户实际只需要「我在哪个仓库」这一级信息。
3. **模型身份无固定渲染点**：模型名与 provider 混在 header，思考强度只在部分状态下可见，
   「当前用哪个模型思考」在最需要确认的时刻（换模型后）反而找不到稳定位置。

## 候选方案与拒绝理由

- **A. 保持现状，只把拥挤行加宽/换行**：拒绝——底面行是常驻行，换行会破坏 ADR 0042 R5/R7 的
  `输入行 + 1 行呼吸 + 底面行` 三段几何，且不解决「一行两义」。
- **B. 路径与指标都留在底部，header 只放品牌**：拒绝——底部行要承载一次性提示 / 计划横幅 / 排队反馈，
  再叠加指标会在流式期与提示叠加时互相吃掉。
- **C. header 左侧显示完整绝对路径**：拒绝——长路径必然截断，截断后的绝对路径比「`~/<名>`」信息量更低。
- **D. 每帧现算家目录（`std::env::var("HOME")`）**：拒绝——见「决策 3」的真实缺陷：同进程内
  `set_var("HOME")`（测试隔离、嵌入运行）会让 header 在帧间抖动。
- **E. 采用方案**：header 左 = 品牌 + `~/<文件夹名称>`，header 右 = 数值指标簇；
  composer 下最后一行 = 左槽提示、右槽模型身份；家目录快照在启动时冻结一次。

## 决策

1. **header 单行双区**：左区 `PONY  ·  ~/<文件夹名称>`（品牌 `ACCENT`+`BOLD`，分隔与工作区 `MUTED`），
   右区数值指标簇（`metrics_segments` 以 ` · ` 连接的单一真值）。右侧无值时为**空串**
   （沿用 v3 R11「无值不显示、不出占位符」）。
2. **宽度三档降级**（不换行、不破几何）：
   - `pad > left_w + right_w`：全量展示，中间补空格右对齐；
   - `pad > left_w + 12`：右侧指标按可用宽度 `ellipsize_width` 截断，**保留指标簇而非整块丢弃**；
   - 更窄：只保留品牌 + 工作区，指标让位（信息优先级：身份 > 数值）。
3. **工作区标签纯函数化** `workspace_header_label_with_home(workspace, home) -> String`：
   家目录本身 ⇒ `~`；家目录之下（严格按路径分隔符判界，防 `/home/dmx` 伪装 `/home/dm`）⇒ `~/<最后一级>`；
   家目录之外 ⇒ 裸文件夹名（**不伪造 `~/`**）；无文件名 ⇒ 完整 display 路径。
4. **家目录快照冻结**：`TuiState::home_dir: Option<String>` 由 `detect_home_dir()`（`$HOME` → `$USERPROFILE`）
   在 `TuiState` 构造时写入一次；`run_tui` 与预览入口显式注入。**任何渲染路径都不得再读环境变量**。
   理由：header 折叠判据必须是帧间不变量；同进程内 `set_var("HOME")` 曾导致相邻两帧
   「`~/x` ↔ `x`」跳变，使逐格快照断言随机失败（真实缺陷，本次修复）。
5. **`TuiState::default()` 不读 env**：`home_dir: None`（工作区显示裸文件夹名），沿用 `model_preferences` /
   `tui_preferences` 的「Default 可复现、不做 IO」约定，避免 `default()` 依赖宿主环境。
6. **底面行单一职责**：composer 下方最后一行 = 左槽（一次性提示 > 计划模式横幅 > 排队反馈，按既有优先级）
   + 右槽（模型身份：`model`，有思考强度时 `model · 思考:<档位>`；**不含 provider**）。
   工作区路径全屏**只在 header 出现一次**。
7. **底面行生命周期契约**：浮层态（`/sessions` 等）按 ADR 0042 §决策 3 收起 composer 与底面行，
   此时最后一行必须是**空画布行**（不得残留 cwd 或模型身份）；关闭浮层后模型身份必须逐字恢复。
8. **底色归属不变**：底面行落在画布底（`theme::CANVAS`），不在 composer 色带内（ADR 0042 R7 不变）。

## 影响

- **代码面**：`crates/pony-agent-core/src/bin/tui.rs`——`TuiState::home_dir`、`detect_home_dir()`、
  `workspace_header_label_with_home()`、`header_metrics_text()`、`status_line()` 右槽、header 三档拼装、
  `run_tui` / 预览入口注入 home。
- **纯本地渲染**：无网络 IO、无 LLM 调用、无后台任务、无对外接口、无持久化 schema 变更。
- **既有契约**：ADR 0042（composer 几何 R5/R7）、ADR 0038（单色层级与沟槽留白）、ADR 0033（状态条出流）、
  ADR 0043（消息层级）均未改写；仅「底面行内容」与「header 内容」两处语义重排。
- **失效的旧断言**：`末尾行左侧必须是折叠 cwd`、`指标行左侧 = cwd` 两类判据作废，改为
  「header 含工作区名」+「末行 = 模型身份」+「浮层态末行为空」。
- **§3 观测性**：结构性 N-A（纯渲染层重排，无新增对外接口 / 网络 IO / 后台任务 / LLM 调用），
  不伪造 `/metrics`。

## 验收

| 契约 | 断言位置 |
| :-- | :-- |
| C1 header 含工作区标签且不混入 composer 提示符 | `tui_render_layout::layout_invariants_hold_on_standard_terminal` |
| C2 最后一行 = 右对齐模型身份（`trim_end` 逐字相等） | 同上 |
| C3 底面行底色 = 画布底、三段几何（输入行 + 呼吸行 + 底面行）不变 | 同上（含 ADR 0042 断言） |
| C4 浮层态最后一行必须是空画布行 | `tui_render_layout::panel_renders_inside_screen_and_keeps_footer` |
| C5 关闭浮层后模型身份行逐字恢复 | 同上 |
| C6 `~/<名>` / `~` / 裸名三种折叠形态，且不误判前缀相近目录 | `tui_render_layout::header_workspace_label_is_home_folded_folder_name` |
| C7 指标段锚定 header(row0) | `tui_overlay_cancel_acceptance::o1_*` |

回归：`tui_overlay_cancel_acceptance`（o1–o8 全套）、`tui_terminal_restore`、`tui_enter_key_wave40`、
`tui_six_fixes_acceptance`、`tui_process_fold_acceptance`。

## 关联

- 修订 ADR 0042 的「底面行 = cwd + 指标」内容契约（几何与呼吸行不变）；
- 与 ADR 0079（过程折叠 / 设置面板）、ADR 0080（启动期零 I/O 探测）同波次落地，共享
  「启动期一次冻结、渲染期不做 IO」的口径。
