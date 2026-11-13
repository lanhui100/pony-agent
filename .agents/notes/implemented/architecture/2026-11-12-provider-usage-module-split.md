# Agent Note: provider 子模块边界——usage 解析归位 `provider/usage.rs`

Status: implemented

## Problem

`crates/pony-agent-core/src/agent/provider/mod.rs` 是 provider 家族的单一入口文件，长期在
file-size 门禁的 grandfather 名单里（基线 ≤9768 行）。本轮「TUI 右上角指标诚实化」在
`extract_openai_usage` 内补了 DeepSeek 原生 `prompt_cache_hit_tokens + prompt_cache_miss_tokens`
分母还原路径（+43 行），把该文件推到 9811 行，越过基线 43 行 —— 门禁报「基线内恶化」。

这不是「性能/正确性」问题，而是**归属问题**：usage 解析是 OpenAI 家族线上协议的一小块
纯函数（输入 `serde_json::Value`，输出 `TokenUsage`），却与协议调度、SSE 状态机、模型目录
等 9000 余行的东西挤在同一文件里。指标口径后续还要动（reasoning 归一、miss 桶消费），
若不先立边界，每次改动都要在同一文件里再借几十行。

## Decision

1. 把 `extract_openai_usage` 整体下沉到新文件 `crates/pony-agent-core/src/agent/provider/usage.rs`，
   由 `provider/mod.rs` 以 `mod usage; pub(crate) use self::usage::extract_openai_usage;` 保持
   既有 `super::extract_openai_usage` 调用点（`openai_sse.rs` / `responses_api.rs` / 本文件）一字不改。
2. 语义零变更：缓存命中三路来源（`prompt_cache_hit_tokens` → `input_tokens_details.cached_tokens`
   → `prompt_tokens_details.cached_tokens`）的优先级、`normalize_token_usage` 收口、
   `USAGE_ORIGIN_PROVIDER` 标记、`None` 语义全部逐字保留；唯一新增是 `prompt_tokens` 缺席时
   用 hit+miss 还原分母（两桶俱在才算，见
   `.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md` §决策 4）。
3. 边界判据（后续同类改动的裁决口径）：**只依赖 `serde_json::Value` 与 `TokenUsage`、无 IO、
   无状态**的线上协议解析，一律进子模块；调度、重试、SSE 状态机留在 `mod.rs`。

## Alternatives considered

- **抬基线（`npm run size:check:update`）**：最省事，但会把「本 lane 新增 43 行」转正为合法体量，
  下一次改动继续挤，直到文件彻底不可维护；且门禁只剩计数器语义。
- **就地加 `#[allow(clippy::too_many_lines)]` 或拆函数**：拆函数不减文件行数（搬运本就是行数问题），
  allow 属第二套判据，门禁不认。
- **把 `TokenUsage` 结构体也一起搬走**：会扩大改动面到 crate 内所有 usage 消费者与 serde 契约，
  与「归位一块纯函数」的最小必要闭包不成比例。
- **把整个 openai 线协议（SSE、delta 提取、工具名归一）一次性拆出去**：超出本次回归基线所需，
  且正与并行 lane 的编辑面重叠，留待单独决策。

## Consequences

- `provider/mod.rs` 9811 → **9754**（≤9768，门禁该项恢复绿）。除以本条搬出的 `extract_openai_usage` 外，
  同批把 `normalize_token_usage`（usage 归一，20 行）与 `extract_chat_delta_text` /
  `openai_safe_tool_name`（OpenAI 兼容线协议的两个纯函数，37 行）按同一判据搬进子模块：
  `provider/usage.rs`（118 行，含用例）与 `provider/openai_wire.rs`（45 行）。
  三者均以 `pub(crate) use` 原位 re-export，父文件调用点零改动。
- `cargo check -p pony-agent-core` 退出码 0；`cargo test -p pony-agent-core --lib` **1523 passed /
  0 failed**（含新增用例 `openai_usage_falls_back_to_cache_hit_plus_miss_when_prompt_tokens_absent`）。
- 并行 lane 的增量已按同一纪律就地收口：那条 lane 在 `provider/mod.rs` 追加了 525 重试分类用例
  （约 +86 行，一度把文件推到 9842 行），本次把这批增量之外的可搬单元（见上）搬走，使文件回到 9754；
  525 用例本身**未搬动、未改写**（`grep -c http_525_enters_retry_loop_and_succeeds_on_second_attempt`
  仍命中 1），其作者可继续在原位迭代。
- 复盘（19:56 复测）：该 lane 仍在写入 `provider/mod.rs`，文件已被带到 **9772**（超基线 4 行）。
  本条给出的边界判据继续有效，但「先拆再写」的纪律需要在该 lane 侧执行 —— 本次不再代其搬运，
  以免与其未完成的重试重构互相踩踏。
- 后续若再动 usage 口径，只需改 `usage.rs`，`mod.rs` 行数不再随口径演进增长。
