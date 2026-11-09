# Agent Note: TUI 底面行思考档位改为裸英文标签

Status: implemented

配套决策：局部修订 [ADR 0078](../../../docs/decisions/0078-tui-header-workspace-and-metrics-layering.md) 决策 6
（底面行右槽文案口径）。

## Context

底面行右槽承担「当前用哪个模型、以多强的思考在跑」，是常驻窄行（宽度受限时还要给排队指示器让位）。
原口径 `model · 思考:高` 有两个冗余：`思考:` 前缀说明了「这是思考档位」，而右槽只有模型身份一个语义；
中文映射（低/中/高/极高）与 provider 契约档位（`low`/`medium`/`high`/`max`）各存一份，多一层翻译负担。

## Decision

1. `TuiState::reasoning_effort_label()` 返回 provider 契约里的英文档位
   （`low` / `medium` / `high` / `max`，`None` ⇒ `default`），删除中文映射。
2. `status_line()` 右槽拼装由 `model · 思考:<档位>` 改为 `model · <档位>`；无思考强度时仍只显示 `model`。
3. 模型/设置面板的类型化提示（`reasoning_effort_display()`，如 `高 · high`）保持不变 ——
   那里是一次性说明性文案，宽度不受限且需要中文语义。
4. 门禁：`tests/tui_metrics_snapshot.rs::c5_bottom_row_reasoning_effort_label_is_plain_english`
   逐档断言底面行以 `model · <英文档位>` 结尾且不含「思考」。

## Alternatives considered

- **A. 裸英文档位**（采纳）：最短、与 provider 请求参数同词，跨语言读者与日志对照零翻译。
- **B. 保留 `思考:` 前缀只换英文（`思考:high`）**：仍多 5 列，前缀在单语义右槽里是纯冗余。落选。
- **C. 改中文为「高」但去前缀（`model · 高`）**：更短，但与 `low/medium/high/max` 契约词不一致，
  用户切模型时看面板 `高 · high`、看底面 `高`，仍需脑内映射。落选。
- **D. 只在窄终端降级为英文、宽终端保留中文**：两套渲染分支意味两套快照断言，收益不抵复杂度。落选。

## 后果

- 底面行宽度占用减少（`高` → `high` 视觉宽度相同但少 5 列前缀；`极高` → `max` 反而更短）。
- 中文读者需要认识 `low/medium/high/max`；面板与 `/reasoning` 提示仍给中文+英文对照，可自解释。
- 撤销点：回改 `reasoning_effort_label()` 与 `status_line()` 两处，并同步 ADR 0078 决策 6 与 c5 断言。
- 验收：`cargo test --test tui_metrics_snapshot`（含 c5）、`cargo test --test tui_render_layout` 全绿。
