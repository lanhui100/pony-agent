# Agent Note: 轮播相位必须显式钉住——TUI 快照断言的确定性前置

Status: implemented

## Problem

`tests/tui_metrics_snapshot.rs::c5_bottom_row_reasoning_effort_label_is_plain_english` 长期随机红
（B118，实测 `cargo test --test tui_metrics_snapshot c5_` → 退出码 101，断言 `tests/tui_metrics_snapshot.rs:300`
失败，实际底面行为 `权限 · 工作区`）。

根因不在被测逻辑，而在**夹具不确定性**：`tui.rs::status_carousel_phase` 在
`status_carousel_started_at = None`（新建 `TuiState` 的默认值）时退化为**墙钟毫秒 % 6000**，
于是底面行右槽落在「模型身份」半周期还是「权限 · <级别>」半周期完全取决于跑测试的那一刻。
断言「底面行以 `model · <档位>` 结尾」就有约一半概率红 —— 且红相里看不出任何产品缺陷。

同类陷阱此前已在 `tests/tui_dialog_wave39.rs` 与 `tests/tui_render_layout.rs` 被踩过一次，
两处都写了 `pin_carousel_phase` 辅助函数并在注释里写明「为什么不钉会随机红」，
但**这条纪律没有跨文件生效**：`tui_metrics_snapshot.rs` 又新写了一个断言底面行的用例。

## Decision

1. 在 `tests/tui_metrics_snapshot.rs` 补 `pin_carousel_phase(state, model_phase)`：偏移量取自产品常量
   （`STATUS_CAROUSEL_MODEL_HOLD_MS` / `STATUS_CAROUSEL_PERM_HOLD_MS` / `STATUS_CAROUSEL_PERIOD_MS`，
   不写死毫秒数），钉完后**当场断言相位已生效**，避免夹具失效后静默假绿。
2. `c5` 在断言前钉在模型相位；其余用例保持相位无关断言（`c1` 只检查底面行不含指标 token，
   两个相位都成立，无需钉）。
3. 纪律（后续同类断言的动作）：**任何断言右槽/底面行具体文本或字色的用例，必须先钉相位**；
   只做「不含某类子串」的否定断言可以不钉。

## Alternatives considered

- **改产品代码：把 `None` 默认成模型相位（确定性）**：能一次性消除所有调用方的陷阱，但会改变
  生产渲染行为（当前设计是「无起点 ⇒ 按墙钟连续轮播」，ADR 0119 的时分交替语义），
  修一个测试夹具的代价不该是改产品时序模型。
- **把 `c5` 改成相位无关断言**（只断言「模型名或权限词出现」）：断言会失去对「档位标签格式」的
  锁定力，正是这个用例要守的东西。
- **删掉该用例**：丢掉的是一条约定的守卫（ADR 0078 §决策 6：档位用英文短标签、不加「思考:」前缀）。
- **加 `#[ignore]` 或重试**：把不确定性藏起来，CI 变成掷骰子。

## Consequences

- `cargo test --test tui_metrics_snapshot c5_` 退出码 0（`1 passed; 132 filtered out`）；
  该用例现在对时间无关，可反复跑。
- 代价：`pin_carousel_phase` 在三个测试文件里各有一份（各文件是独立 bin-as-module 测试目标，
  无法共享 helper）；重复是可见的，但比「随机红」便宜。
- 遗留：`STATUS_CAROUSEL_*` 常量已成为测试夹具的公开依赖，改周期/持有时间会同时影响
  三个测试文件的夹具（偏移量按常量比例计算，故不会失效，只是断言文案里的毫秒数会变）。
