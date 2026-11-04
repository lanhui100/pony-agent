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
