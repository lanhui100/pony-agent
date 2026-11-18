# Agent Note: 上下文占用改为「下一跳预估」（projectedTokens）

Status: implemented

工单：B116。影响文件：`agent/session/usage.rs`（ContextSurfaceLedger + 估算纯函数）、
`bin/tui.rs`（投影账本接线与 header 渲染）。

## Problem

TUI/桌面 上下文% 取最近一次调用已结算的 `context_input_tokens`。采样描述的是**上一次请求**：
工具结果刚追加、下一跳尚未返回 usage 的窗口期内读数冻结，用户直觉却是「现在还剩多少」；
compaction 之后也要等下一次采样才下降。

## Decision

对齐 DSH `packages/llm/token-meter/src/usage-projection.ts`：

`projectedTokens = max(0, pressureTokens + surfaceTokens − sampledSurfaceTokens)`

- `pressureTokens`：最近一次已结算调用的 prompt 侧压力（协议归一 `context_input_tokens`，
  缺失回退裸 `input_tokens`；0 视为无采样）；
- `surfaceTokens`：请求表面累计估算（追加 +、替换/压缩 −），估算**只**走
  `compression::estimate_text_tokens` 同一出处（每 4 字符 ≈ 1 token；中文按字符不按字节），
  消息封装开销 4 token 单点定义；
- `sampledSurfaceTokens`：采样时刻的表面锚点；
- 工具往返按 `arguments + result` 两条消息估算，按 activity id 去重（`turn:tool` 与
  `turn:tool_result` 双通道只计一次）；`status=running/planned` 不计（下一跳尚未成型）；
- 消费：header `↑in` 与 `上下文 x%` 优先用投影值（窗口已知时按 `context_ratio_for_window`
  重算；缺窗口回退快照比）；无采样时回退现行口径（`None`，绝不用 0 冒充空上下文）。
- **持久化 usage 真值不动**：账本是 UI 侧瞬时态，会话切换/`/clear` 复位。

## Alternatives considered

- **A. 每次工具结果后主动估算并覆盖 last_context_tokens。** 落选：会污染持久化/台账语义，
  且下一次采样前无法区分「估算」与「真值」。
- **B. 用字节数 / 4 估算。** 落选：中文一字三字节，会系统性高估三倍；复用压缩侧字符估算器，
  保证「按 A 判压缩、按 B 估投影」同口径。
- **C. 等下一次 usage 采样再更新（现状）。** 落选：工具密集期读数冻结，正是本卡要修的用户体感。

## Verification

- `cargo-lane test --lib -- usage`：51 passed（投影上升 / compaction 下降且非负 / 无采样回退 /
  重复采样幂等）；
- `cargo-lane test --bin pony -- context`：12 passed（header 投影接线）；
- `npm run rust:lint`：usage.rs/tui.rs 0 新增；`size:check`：两文件 0 新增。
