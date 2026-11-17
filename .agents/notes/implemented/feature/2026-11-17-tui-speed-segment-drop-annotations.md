# Agent Note: TUI 速度段数值后不再挂注释文字（去 `e2e` / `剔突发N`）

Status: implemented

## Problem

用户看 TUI header 右上角的速度段，读到的不是「这一轮多快」，而是一串需要解释的附注：

```
↑25k ↓5k · 缓存 60% · 上下文 25% · ≈120 t/s e2e · 首token 800ms
↑25k ↓5k · 缓存 60% · 上下文 25% · 120 t/s 剔突发2 · 首token 800ms
```

`e2e` 是英文口径标签，`剔突发N` 是末端样本量说明，两者都挂在**速度数值之后**，对 glance
（扫读）场景没有可行动信息：用户只需要知道「多快」。它们还占着 header 右槽宽度，与
`首token` 抢位置——这正是 ADR 2026-11-14-tui-header-metrics-overflow 记录的让位压力的来源。
前案 ADR 2026-11-15-tui-speed-segment-drop-inline-cadence 已删掉 `decode` 与 `n=`，但把
`≈e2e` 与 ` 剔突发N` 作为「异常口径标记」保留了下来；用户本次明确要求把数值之后的注释一并去掉。

## Decision

1. **速度段只出「值 + 单位」**：`format_speed_segment` 收敛为 `(speed, prefix)` → `"{值} t/s"`，
   删除 `basis` / `burst_excluded` 两个形参；`stream_decode`、`end_to_end`、`basis` 缺席一律同形。
2. **`≈` 前缀保留**：它修饰数值本身（约数），不是数值之后的注释文字；`end_to_end` 仍以
   `≈` 自描述「不可比」，但不再追加 `e2e` 标签。这是与旧条「去掉 `e2e` 会让 `≈` 无解释」的
   明确改判：header 是扫读位，`≈` 是通用数学符号，解释责任移交给桌面 tooltip 与 trace。
3. **数据面不动**：`turn_speed_basis` / `turn_calls` / `turn_burst_excluded` 仍是快照字段，
   桌面 tooltip（`formatSpeedSampleLabel`）与 trace 照旧可取证；本次只改 TUI 行内呈现。
4. **回归锁定**：`crates/pony-agent-core/tests/tui_header_metrics_overflow.rs` C2 的字面量由
   `120 t/s 剔突发2` 改为 `120 t/s`，夹具 `turn_speed_basis` 改为 `EndToEnd`（让 `≈` 前缀与
   `e2e`/`剔突发` 两条旧后缀同时处于可触发状态），并新增反向钉子（header 全文不得出现
   `e2e` / `剔突`）；其余让位契约断言原封不动。

## Alternatives considered

- **A. 只删 `剔突发N`，保留 `e2e`**：`e2e` 正是用户指出的「英文注释放错位置」，删一半仍留下需要解释的文本；否决。
- **B. 保留异常口径标记，只在宽度不足时隐藏**：同一读数在不同终端宽度下形态不同，截图/日志无法互认，
  且让位策略再长一档；否决（同旧条方案 C 的否决理由）。
- **C. 把口径/样本量移到紧跟速度的 header 第二行**：破坏 ADR 0042 / ADR 0078 的单行 header 几何；否决。
- **D. 连 `≈` 前缀一并去掉（纯 `120 t/s`）**：更彻底，但 `≈` 属于数值修饰而非注释文字，且是跨口径可读性的
  最小诚实标记；用户本条只要求「速度后的注释文字」；保留。若后续仍嫌 `≈` 多余，再按呈现层变更单独立条。
- **E. 采用：删 `e2e` 与 `剔突发N` 两个后缀，保留 `≈` 前缀**（本 ADR）。

## Consequences

- 正面：速度段宽度再压 4~10 列，`首token` 在中等宽度下的让位压力进一步下降；header 只剩读数。
- 负面/已知限制：TUI 内不再显示速度口径与突发剔除数——需要区分 `stream_decode` / `end_to_end`
  或查剔除数时走桌面 tooltip 或 trace。
- 契约面：旧条 `2026-11-15-tui-speed-segment-drop-inline-cadence` 的第 1、2 项保留理由被本条取代
  （`≈e2e` 与 `剔突发N` 不再上屏），该条已链回本条。
- 观测性：结构性 N-A（纯呈现层文本裁剪：无新增对外接口 / 网络 IO / 后台任务 / LLM 调用，不新增
  metrics 归属与收口点）。

## Related

- 前案：`implemented/feature/2026-11-15-tui-speed-segment-drop-inline-cadence.md`（本条取代其保留项）
- 前案：`implemented/bug-fix/2026-11-14-tui-header-metrics-overflow.md`（header 右槽宽度与让位）
- 前案：`implemented/bug-fix/2026-11-13-token-speed-evidence-and-window-purity.md`（速度口径真值源）
