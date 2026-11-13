# Agent Note: 规模债复发的模块拆分（provider 测试模块 / turn_stream 收口调用面 / tail 刷帧）

Status: implemented

## Problem

`B123` 于 2026-11-13 01:26 以「provider/mod.rs = 9768、runtime/turn_stream.rs = 3556」闭环。此后并行 lane 继续向两处追加：

- `provider/mod.rs` 回涨到 **9912**（基线 ≤9768，+144）；
- `runtime/turn_stream.rs` 回涨到 **3646**（基线 ≤3578，+68），其中 `handle_stream_tool_turn` **1598** 行（基线 1588）。

按 `.agents/notes/implemented/process/2026-11-12-file-size-gate-module-split-not-baseline-bump.md` 的纪律：**抬基线（`size:check:update`）被禁止**，只能拆模块或做行中性化。此前该纪律已被用过一次（B123 自身靠行中性化 + F3 重构回落），这次回涨说明「靠事后行中性化」不是可持续的解——文件里存在**整块可外移的单元**，应当一次性拆出去。

## Decision

三处纯搬迁（不改任何逻辑、不改调用点语义），外加一处测试夹具隔离：

1. **`provider/mod.rs` 的单元测试模块外移** → `provider/tests.rs`，`mod.rs` 只留 `#[cfg(test)] mod tests;`。模块身份不变（仍是 `provider::tests`，`use super::*` 指向 `provider`）。文件 9912 → **6956** 行（−2956）。
2. **流式轮的失败/取消收口调用面外移** → `runtime/turn_stream_failures.rs`：`fail_stream_turn_gated_dispatch{,_pending,_tagged}`、`cancel_stream_turn_for_stop` 四个「过闸门 + 参数转发」薄方法改为 `pub(crate)` 放进独立 `impl AgentRuntime` 块。`turn_stream.rs` 3646 → **3448** 行（−198）。
3. **followup 收尾的 tail delta 刷帧外移** → `stream_support::flush_tail_delta_frames`（自由函数，入参 6 个：sink / coalescer / turn_id / session_id / step / now_ms）。`handle_stream_tool_turn` 1598 → **1572** 行（−26，回到基线 1588 以下）。
4. **测试夹具隔离（伴随修复）**：`build_request_truncates_older_history_to_context_window` 的偶红根因不是并发 goal/todo（此前已用专属会话 id 隔离过一次），而是 **semistable 段的绝对体积取自进程级全局发现**（用户级 AGENTS.md / 全局技能清单来自 home 与环境变量）。并行跑全量 lib 单测时该体积可变，于是 1700 token 窗口下的截断点漂移。修法：①会话 cwd 指向专属空目录；②把「最近一轮必须在场」的绝对断言换成**截断方向不变量**（旧轮保留 ⇒ 最近轮必须保留；旧轮消失 ⇒ 必须有截断提示），断言不再依赖全局发现体积。

## Alternatives considered

- **抬基线（`npm run size:check:update`）**：直接被既有 ADR 的纪律禁止；这会把「自己制造的增长」变成对后人的既成事实。
- **继续行中性化（合并短 `let` / 压平函数签名）**：能让门禁变绿，但门禁本意是控制复杂度；且 B123 的复发证明这种一次性手段不可持续——真正该做的是把可外移单元拆走。
- **拆 `handle_stream_tool_turn`（1572 行）为多函数**：函数内注释已写明「预/后阶段共享 20+ 个可变局部状态，并刻意用函数内宏保证逐调用语义等价」，强行拆成方法/上下文结构体会把语义等价藏进重构本身。本轮只搬走唯一自包含的 tail 刷帧块，其余保持原样，并在残余中登记。
- **把 provider 的 `collect_anthropic_sse_message_from_response`（313 行，基线 311）也拆掉**：它与 SSE 解析状态机耦合，改动风险高于收益；本轮目标是把**文件级**两个违规清零，函数级债留待单独评审。
- **改测试用 EnvGuard 隔离 home 目录**：环境变量是进程级全局，并行测试下写 env 会引入新的跨用例竞态（既有 wave42 用例已在冒这个风险）；改为「断言相对不变量」更稳且不欠新债。

## Consequences

- `provider/mod.rs` 6956（≤9768）、`runtime/turn_stream.rs` 3423（≤3578）、`handle_stream_tool_turn` 1572（≤1588）：三项全部回落基线内，`size:check` 0 new violation（其余违规来自其它 lane）。
- 纯搬迁零行为变化：lib 全量 1535 passed；`wave62_tool_cancel` 7 passed、`wave61_team_activation` 16 passed。
- 新增两个模块文件（`provider/tests.rs`、`runtime/turn_stream_failures.rs`）与原文件同属既有包，不新增公共 API。
- 残余 1：`handle_stream_tool_turn` 仍有 1572 行，是本次范围外的结构性债；真正拆解需要先把「预/后阶段共享状态」提炼成显式上下文对象，属独立提案量级。
- 残余 2：`collect_anthropic_sse_message_from_response` 313 行（基线 311）仍在函数级违规边缘，随时可能因并行 lane 追加而红。
- 观察：size 债在 12 小时内两次复发（B123 → 本条的 B145），说明「并行 lane 追加 + 只做事后收敛」的模式本身需要节流（例如提交前本地跑 `size:check`），已在 backlog 描述里记录，未在本条展开。
