# Agent Note: TUI header 指标溢出改为整段让位（首 token 不再被截掉）

Status: implemented

## Problem

用户报：TUI 右上角速度段（`120 t/s decode n=1 剔突发2`）出现后，`首token <ms>`
从屏幕上消失，有时只剩 `首t…` 这种半截字符。

根因不在 `decode` 标签本身（它是 F1 I4 要求的口径/样本量自描述），而在 header 的
溢出策略：`bin/tui.rs::ui` 中等宽度分支对**整簇**调用
`ellipsize_width(&right_meta, pad - left_w - 1)`，从右往左截断。`metrics_segments`
的书写顺序是「会话累计/比率 → 轮级实时读数」，`首token` 恰好在最右端，于是：

- 速度段变长后（新增 ` decode n=<样本量>` / ` 剔突发N`），最先被砍的就是 `首token`；
- 截断按**字符列**进行，会把一段切成 `首t…`，既丢信息又不可读；
- 空出来的宽度仍然显示低优先级的累计值，信息价值倒挂。

典型终端（workspace `~/pony-agent`、110→96 列）下 `首token` 整段消失；
60~96 列区间可稳定复现（回归测试见
`crates/pony-agent-core/tests/tui_header_metrics_overflow.rs`）。

## Decision

`bin/tui.rs` 新增 `header_metrics_text_within(state, budget)`，把 header 右槽的宽度降级
从「整簇字符截断」改为「**按段优先级整段让位**」：

1. 放得下 ⇒ 原样返回（行为与 ADR 0078 全量展示档一致）。
2. 放不下 ⇒ 以 `MetricsKind` 为序丢弃**整段**（不切半段、不留悬挂 ` · `）：
   `缓存 → 上下文 → ↑in ↓out → 速度 → 首token`；`首token` 最后退出。
3. 只要发生让位，就在簇首加 `… ` 自描述；被让位段不再以任何形式残留。
4. `（上轮）` 是簇级诚实性后缀，固定在右端、不参与让位；没有可修饰的读数时整簇不渲染
   （不输出孤立免责声明）。
5. 极端窄屏连最后一段都放不下时，才退回 `ellipsize_width` 做段内省略——此时省略号
   自身即让位标记，不再叠加 `… `。

`metrics_segments` 的返回类型同步从 `Vec<String>` 收敛为 `Vec<MetricsSegment>`
（文本 + `MetricsKind`），让「渲染顺序」与「让位优先级」成为两个正交事实，避免未来
再加指标段时靠字符串前缀猜优先级。

## Alternatives considered

- **A. 缩短速度标签（`decode n=1` → `d/1`）**：直接违反 token-speed v2 I4 的
  「口径 + 样本量必须可辨识」，且治标不治本（样本量位数增长后仍会溢出）；否决。
- **B. 继续整簇截断、把 `首token` 移到最左**：会让 `首token` 在中等宽度下优先保留，
  但破坏 header 的既有阅读顺序（累计 → 比率 → 实时），且其他段仍可能被切成半截；否决。
- **C. 只从最左整段丢弃（纯尾部保留）**：实现最简，但会让 `↑in ↓out` 在 60 列时最先消失，
  与既有 `tui_dialog_wave39::a10` 契约（窄终端 header 必须仍承载 ↑/↓）冲突；否决。
- **D. 指标改第二行 / 换行**：破坏 ADR 0042 R5/R7 与 ADR 0078 的单行 header 几何；否决。
- **E. 采用方案**：整段让位 + 显式优先级 + `… ` 前缀（本 ADR）。

## Consequences

- 正面：`首token` 与速度段在中等宽度下稳定可见；任何段要么完整、要么整段让位，
  不会出现 `首t…` 这种伪读数；让位有 `… ` 可见证据。
- 负面/已知限制：60~96 列区间会先让位缓存/上下文（静态比率），需要时用户拉宽窗口即可
  看全；极端窄屏（< 约 44 列）仍允许最后一段段内省略。
- §3 观测性：结构性 N-A（纯呈现层重排：无新增对外接口 / 网络 IO / 后台任务 / LLM 调用；
  metrics / logs 的「无常驻监听地址、无 /metrics」判定不变，不新增 applicable /
  enforcement_point），已同步 `docs/observability.yaml` 的归属说明。
- 验收：`crates/pony-agent-core/tests/tui_header_metrics_overflow.rs`（C1/C2/C3）+
  既有 `tui_dialog_wave39::a10`、`tui_metrics_snapshot`、`tui_metrics_dual_caliber`、
  `tui_render_layout`、`tui_token_context_wave41` 全绿。
