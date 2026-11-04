# 设计：一次响应多工具调用的批量执行（P1-1）

> 结论先行：**本轮只落「解析层复数接缝 + 消除静默截断」**；批量执行需独立一波 + L3 复核（原因见 §4）。

## 1. 现状与依赖图（file:line）

```
上游响应解析
├─ OpenAI 非流式
│  ├─ provider/mod.rs:3230  pub fn extract_openai_tool_calls(&Value) -> Vec<ToolCall>   ← 本轮新增
│  ├─ provider/mod.rs:3251  fn warn_on_dropped_openai_tool_calls(...)                   ← 本轮新增
│  │    （告警行组装：:3262 openai_dropped_tool_calls_warning，纯函数可单测）
│  │    调用点：:1175 openai_followup_sync / :1893 openai_decision（仍只取首个）
│  └─ ProviderDecision.tool_call: Option<ToolCall>（单数，字段不可扩）
└─ Anthropic
   ├─ provider/mod.rs:3487  pub fn extract_anthropic_tool_calls(&[Value]) -> Vec<ToolCall> ← 本轮新增
   └─ provider/mod.rs:3512  fn ensure_single_anthropic_tool_call(...) -> Result<(),String>
        └─ 多调用仍 Err（:1309 / :2071 两处调用点），本轮只丰富诊断信息

执行与回填（未改，批量化的真正难点）
├─ runtime/turn_stream.rs:110  handle_stream_tool_turn(...)：单 `current_tool_call` 循环
├─ runtime/turn_sync.rs:438    run_turn(...)：同步路径同构的单一 tool_call 流
├─ provider/mod.rs:3550        openai_assistant_message_for_tool_result(...)  ← 1:1 形态
└─ provider/mod.rs:4351        anthropic_assistant_content_for_tool_result(...) ← 1:1 形态
```

## 2. 批量回填的**正确形态**（落地时必须照此实现）

- **OpenAI**：一条 `assistant` 消息携带**全部** `tool_calls`（顺序与执行顺序一致），随后**每调用一条**
  `role:"tool"` 消息（`tool_call_id` 对应）。禁止把 N 条 tool_result 塞进同一条消息。
- **Anthropic**：一条 `assistant`（原始 content blocks，含全部 `tool_use`，`id` 不变）+ **一条** `role:"user"`
  消息（`content` 为 N 个 `tool_result` block）。
- **红线**：沿用现有 1:1 helper（`anthropic_assistant_content_for_tool_result`）拼 N 次会得到 N 条
  assistant 消息 / N 份 assistant content ⇒ **Anthropic 直接 400**（同内容重复 + tool_use id 失配）。

## 3. 执行语义约定

1. **Ask 必须作批终止符**：批中出现 `ask_user` 时，先执行它之前的调用、暂停并等待宿主应答，
   剩余调用**不得**执行（否则会出现「等待期间仍在改工作区」的不可解释状态）。
2. **并发留开关**：默认**串行**执行（与当前单调用语义一致、错误归因简单）；并发需开关 + 逐调用
   治理闸（`DispatchContext` 的 budget/cancel 已按调用维度 ready，但批内共享 turn 令牌）。
3. **失败策略**：单调用失败不中断同批（工具错误反馈给模型）；熔断/止损闸**按调用计数**（现状，落地时保持）。

## 4. 为什么不在本轮硬塞（Lead 裁决 + 侦察结论）

- 体量：完整最小切片约 **350–550 行 / 4–5 文件**（解析 + 两条执行循环 + 批执行 + 4 个回填 helper），远超单卡边界。
- 最大风险在**回填协议形态**（§2 红线）：一旦形状错，Anthropic 侧 400 是**整轮失败**，不是局部降级；
  必须有独立 L3 复核与真实上游回放证据。
- 本轮已消除**最刺痛的静默行为**：解析层不再丢调用（复数接缝可用 + OpenAI 丢弃时告警 + Anthropic
  诊断可读）。因此后续批量执行可以对着一份「已知会有多调用」的数据面施工，而不是盲改。

## 5. 风险清单

| # | 风险 | 缓解 |
| :-- | :--- | :--- |
| R1 | 批量回填形状错 ⇒ Anthropic 400 / OpenAI 串号 | §2 硬约束 + 上游契约测试（mock SSE 断言 messages 形状） |
| R2 | Ask 混在批中 ⇒ 工作区半改又停顿 | Ask 作批终止符（§3.1），先执行前缀批 |
| R3 | 并发批 ⇒ 工具间竞态（同一文件读写） | 默认串行；并发需开关 + 工具 `concurrent_safe` 白名单 |
| R4 | 熔断/止损闸计数口径漂移 | 明确按调用计数（§3.3），并有单测锁 |
| R5 | 批执行中用户 Stop | 复用 turn 取消令牌（`DispatchContext.turn_cancellation`），逐调用检查 |

## 6. 交接：D2 / D3（2026-11-04，会话上下文耗尽处停下）

**状态**：D1a（`6d743b82`）与 D1b（`a57f636b`）已交付并验证；D2/D3 未开工，工作树 clean。

**D2 定稿签名**（`provider/mod.rs`）：
```rust
pub fn continue_with_tool_results(
    &self, request: &ProviderRequest, tools: &[ToolDefinition],
    accumulated_messages: &mut Vec<Value>, assistant_message: Option<&Value>,
    calls: &[ToolCall], results: &[ToolResult],
) -> Result<ProviderResponse, String>;

pub fn continue_with_tool_results_stream<F>(
    &self, request: &ProviderRequest, tools: &[ToolDefinition],
    accumulated_messages: &mut Vec<Value>, assistant_message: Option<&Value>,
    calls: &[ToolCall], results: &[ToolResult], on_delta: F,
) -> Result<ProviderResponse, String> where F: FnMut(ProviderStreamChunk);
```
分派：长度不匹配 ⇒ 早退 Err（不构造请求）；`calls.len()==1` ⇒ **直接委派**既有 1:1 入口（N=1 回归锁）；
`N>1` ⇒ D1b 构造器组装 messages，走既有 `post_openai_json` / anthropic post 通道（不新写 HTTP 层）。

**写域**：`provider/mod.rs` + `runtime/tests.rs`（并把 `struct MockHttpServer` 提 `pub(crate)`，仅可见性）。

**验收**：① wire 级请求体（OpenAI `role=tool` 恰 N；Anthropic 1 条 user 含 N 个 `tool_result`）；
② N=1 委派等价（既有 1:1 用例全绿）；③ 长度不匹配 ⇒ mock 零请求。

**D3**：`turn_stream.rs::handle_stream_tool_turn` 与 `turn_sync.rs` 同构循环改批执行 —— Ask（`ToolKind::Interactive`）
作批终止符、per-call 熔断闸保持、批后**一次** followup；落地前做一次 Anthropic 真实回放（唯一会 400 的地方）。

## 7. D2 已完成（`8f33eb27`）与 D3 剩余范围

**D2 已交付**：`continue_with_tool_results` / `continue_with_tool_results_stream`（provider/mod.rs:1149/:1236），
三步分派 + 3 条 wire 级测试；`--lib` 1231/0。已知偏差：N>1 流式走同步 post 后回放（非真增量）；
N>1 未接 retry（避免 accumulated 二次追加）。

**D3 剩余（运行时循环批执行）**，建议分两波：
- D3a `turn_sync.rs`（无流式，形态简单）：`current_tool_call: Option<ToolCall>` → 批；
  逐调用执行 + 逐调用 hop 记录；批后**一次** `continue_with_tool_results`；Ask（`ToolKind::Interactive`）作批终止符。
- D3b `turn_stream.rs::handle_stream_tool_turn`（约 1300 行，产品主路径）：同构改写，注意四条 recovery 分支
  （wall budget / followup limit / hop limit / duplicate signature）与两条熔断闸的批语义；真流式增量留作后续。
- 两波都落地后，**删除** `ensure_single_anthropic_tool_call` 的 >1 Err（当前仍 Err，是本目标 D 项未完成的最直观标志），
  并同步翻转 src 单测 `anthropic_rejects_multiple_tool_calls`。
- `parallel_tools` 并发接线仍为独立可选项（开关默认关）；Ask 不可与其它工具同批。

**验收**（两波共用）：① 2 个调用都被执行且顺序保序；② 批后 followup 请求体 1 assistant + N 结果；
③ 单调用路径逐字不变；④ 熔断闸仍按 per-call 计数；⑤ 落地前做一次 Anthropic 真实回放（400 风险点）。

## 8. D3a 已完成（turn_sync 批执行）—— 待提交

- `turn_sync.rs:15` 签名改整批 `Vec<ToolCall>`；批内顺序执行（沿用原治理/沙箱/预算入口，逐调用 hop 与熔断计数）；
  批后**一次** followup（N==1 逐字走 1:1，N>1 走 D2 批量入口）；`sync_tool_call_ends_batch`（`ToolKind::Interactive`）作批终止符。
- `provider/mod.rs` 三条 OpenAI **非流式**入口不再丢弃多余调用（`tool_call` 单数入口逐字不变）——
  不改它 `tool_calls` 恒 ≤1，批执行无从成立，属必要越界。
- 门禁：`--lib` 1233/0、clippy 零新增、lifecycle 7 / stop_contract 7 / promote 9 全绿、audit-diff 0。
- **已知偏差（D3b 前须处理）**：① 非 pending 的交互工具停批后剩余调用直接丢弃（无批内重放）；
  ② 批内第 2..N 个调用未过 `normalize_tool_directive`；③ 重复调用闸只查批内首个（另有 per-call hop 限额硬停）；
  ④ native transcript 仍 per-call 形状；⑤ `anthropic_rejects_multiple_tool_calls` 缺 `#[test]`（HEAD 即 dead_code），D3b 翻转时需补。

## 9. D3b 已完成（流式主路径批执行）

- `turn_stream.rs:120` `handle_stream_tool_turn` 收整批 `Vec<ToolCall>`：批内顺序执行，逐调用保留
  hop/工具事件/hook/两条熔断闸/pending/aborted 语义（仅缩进）；`:706` 交互工具作批终止符；
  `:779` 批后一次 followup；`:1075` 整批出批。
- `turn_flow.rs:3098` `provider_followup_stream_with_calls`（N==1 逐字委派 1:1；N>1 走 D2 批量入口，回放式）。
- `openai_sse.rs` 流式累积器收全批；`provider` 流式 followup/decision 上报 plural。
- 收尾：删除 `ensure_single_anthropic_tool_call` 及 3 处调用；其单测补 `#[test]` 并翻转为
  `anthropic_accepts_multiple_tool_calls`（2 个 ⇒ 返回 2 个）。
- 门禁：`--lib` 1236/0、clippy 零新增、lifecycle 7 / stop_contract 7 / promote 9 / turn_failed 4、audit-diff 0。

**仍待（下一波）**：① Anthropic **流式**与 OpenAI **Responses 流式**解析仍单值 ⇒ 这两路批退化为单调用；
② N>1 followup 非真增量（无 ttft）；③ 批内仅首个过 `normalize_tool_directive`、重复签名闸只查首个；
④ 非 pending 交互工具停批后余下丢弃；⑤ `parallel_tools` 并发接线（本目标第二clause）仍未做。

## 10. D4 已完成（parallel_tools 并发接线，开关默认关）

- 开关 `PONY_AGENT_PARALLEL_TOOLS`（默认关，仅 `1/true/on/yes` 开；未识别 fail-safe 关）。
- `runtime/batch_plan.rs`：`plan_batch_segments`（关⇒每调用一段串行；开⇒连续 `concurrent_safe` 且非交互、段长≥2 才并发）、
  `execute_segment_in_parallel`（`parallel_tools::execute_calls` + `Mutex<Vec<(usize,…)>>` 按指针同一性回收后原序展开，panic 隔离）、
  `execute_tool_call_with_turn_context`（收敛两条路径原本内联的上下文构造）。
- 接入：`turn_sync.rs:294` / `turn_stream.rs:735` 段循环；预/后阶段用函数内宏保持同一段代码文本。
- 测试：5 条运行时用例（峰并发≥2、Run/Ask 在途并发恒 1、关时峰 1、结果与调用一一对应）+ 3 条规划单测；`-- parallel_tools` 12 passed。
- 门禁：`--lib` 1244/0、clippy 零新增、lifecycle 7 / stop_contract 7 / promote 9 / turn_failed 4、audit-diff 0。

**开关开启时的已知差异**：并发段按「整段预→整体执行→整段后」推进（后置调用的 hook/工具事件先于前一调用执行发生；
若某 hook `fail_turn` 则整段不执行）；并发上限 `min(段长,8)`。

**整体收口状态**：A/B/C 全部完成；D 的「多 tool_use 不再一律 Err」与「接入 parallel_tools 可并发子集」均已落地。
遗留（非本目标）：Anthropic/Responses **流式**解析仍单值；N>1 followup 非真增量；批内仅首个过 `normalize_tool_directive`；
non-pending 交互工具停批后余下丢弃；Windows 真机 e2e 未验证。
