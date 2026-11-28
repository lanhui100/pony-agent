# Agent Note: pony-tui/tui.rs 巨石按无状态域分批拆解（Phase 2 首波）

Status: implemented

## Problem

`crates/pony-tui/src/tui.rs` 14,689 行，是审计里排名第 1 的并发冲突热点（近 1 月 97 次修改）。
它同时装着：`TuiState` 结构与其 1,400 行 impl、`UiEvent` 协议枚举、消息区渲染主循环
（`ui()` 1,009 行）、事件循环（`tui_event_loop` 3,349 行）、以及一大批**本可以独立存在**的
无状态纯函数——样式取样、行拼装、字形常量、滚动饱和换算、provider/session 视图模型。

这些纯函数与 `TuiState` 零耦合，却与巨石同居一个文件：改一个 provider 表单字段要打开 14k 行的
文件，多 Agent 并行时必在文件尾互相冲突。`scripts/file-size-baseline.json` 与文件头的
`ponylint:file-size-exempt [until 2026-12-31]` 是这条债的记账凭证。

## Decision

**按「是否依赖 `TuiState`」划第一刀，只切无状态域。** 本波切出三个模块，全部无状态、无 IO、
可在无 TTY 环境独立断言：

| 新模块 | 内容 | 行数 |
|---|---|---:|
| `tui/view_models.rs` | `SessionItem` / `ProviderItem` / `ModelItem` / `AddProviderForm` / `AddProviderPayload` / `gateway_display` 等纯数据与格式化 | 124 |
| `tui/render_text.rs` | `ink`/`muted`/`faint` 样式取样、`wrap_line`/`push_*` 行拼装、字形常量、`layout_input` 列映射 | 282 |
| `tui/scroll_motion.rs` | 契约 F-H 滚动饱和换算（`calculate_scroll_offset`/`clamp_render_scroll`）、契约 F-I 动效帧 | 57 |

切出契约（红线，逐条守住）：

- **符号名 / 签名 / 语义逐字不变**。公开面用 `pub use self::<mod>::*` 原路径重导出，
  外部 `pony_tui::tui::SessionItem` 等导入路径不变；仅跨 `tui` 模块内部使用的符号提为
  `pub(crate)`（`ink`/`wrap_line`/`RAIL` 等），可见性只放宽到原调用方集合。
- **注释与契约说明随代码同行搬运**，不在原处留副本——一处事实一个家。
- `RAIL` / `CHAT_BOTTOM_GAP` / `DRAWER_INPUT_GAP` 等仍被 `ui()` 与事件循环引用，**留在 `tui.rs`**
  并提为 `pub(crate)`，不做无意义的双向搬家。

**本波不碰**：`TuiState` 结构与两个 impl（1,400 行）、`ui()`（1,009 行）、
`tui_event_loop()`（3,349 行）。它们与状态机强耦合，拆解需要先定义渲染器/事件循环的接缝，
属后续独立波次（见 Consequences）。

## Alternatives considered

- **方案 A：只切无状态纯函数，不动状态机（采纳）**。Deletion Test：删掉这层拆分，「找 provider
  表单定义」的路径重新扩散到 14k 行文件，且每次改都要与事件循环改动争同一文件尾；切出后每个域
  自带上下文，代价只是多一层 `mod` 声明。收益/风险比最高：纯函数搬家可用编译器与 301 例
  既有单测完全验证，零行为面风险。
- **方案 B：一次性把 `ui()` 拆成 renderer 模块 + `tui_event_loop` 拆成 input/command 分发器**。
  落选：这两块合计 4,358 行，拆分必须先引入「渲染上下文」与「事件处理器 trait」两层新抽象，
  属于架构变更而非搬家；与纯函数搬家混在同一提交，review 无法分辨「搬错了」与「抽象设计错了」。
- **方案 C：只加豁免期限、不拆**。落选：文件头豁免已写 `until 2026-12-31`，靠延期续命等于把
  债转成永久豁免；且 `check-file-size.mjs` 的 `[until ...]` 方括号语法已被 B1795831357716 封堵，
  这条路本身不再成立。
- **方案 D：按业务域切（ask/plan/goal/team）而非按「是否依赖 TuiState」切**。落选：业务域切法
  会把 `TuiState` 字段访问一起搬走，必须先做状态可见性重构（`pub(crate)` 化 200+ 字段），
  风险面远大于收益，且这正是方案 B 才该回答的问题。

## Consequences

- `tui.rs` 14,689 → 14,270 行。**规模豁免仍在**：本波只切走 3.2%，离 3,000 行阈值很远。
  `npm run size:check` 输出 `0 new violation / 56 grandfathered`，豁免条目未变——**不得**在此
  宣称「已消除豁免」。
- 新增三个模块的改动半径自包含：改 provider 表单只开 `view_models.rs`，改行距只开
  `render_text.rs`，改动效帧只开 `scroll_motion.rs`。
- 已识别的后续波次（均未开工，需独立任务与独立 review）：
  1. `ui()`（1,009 行）→ 按区域（header/chat/composer/overlay）拆渲染器；
  2. `tui_event_loop()`（3,349 行）→ 按输入源（键盘/鼠标/宿主事件/定时器）拆分发器；
  3. `TuiState` 两个 impl（1,400 行）→ 按能力（输入编辑/滚动/面板/会话）拆为多个 impl 块文件；
  4. 上述三步完成后才能删掉文件头豁免并下调 `file-size-baseline.json`。
- 拆分后 `char_width`/`is_no_line_start`/`line_display_width` 的调用从同文件变为
  `crate::tui::text::*` 导入——多了一条显式依赖边，换来的是「排版能力只有一个家」。
