# 契约矩阵 —— TUI Token 与上下文计算对齐 ADR 0035 S1（wave-41）

## 0. 症状与根因（用户反馈：输入 token 和上下文计算有误）
1. **输入 Token 统计口径漂移**：
   - 当前 TUI 将各轮次乃至工具多跳之间的 `input_tokens` 进行跨轮跨调用全局累加（`total_input_tokens += input`）。
   - 用户与系统架构在 ADR 0035（及桌面端状态栏与输入框圆环）中明确裁定：
     **S1 当前上下文输入** := 最近一次已完成模型调用的 `input_tokens`；严禁跨调用、跨轮次求和。
     TUI 底部指标行中的 `↑` 代表当前上下文输入占用（而非历史计费累计），应与桌面端严格同源对齐。
2. **上下文百分比分子计算有误**：
   - 当前 TUI 上下文百分比分子采用 `last_context_tokens = total_tokens`（即 prompt input + completion output 之和），导致上下文占用率虚高，且在长回复时混入生成 token。
   - 桌面端规范：上下文占用百分比 = `当前上下文输入 (current_context_input_tokens) / context_window_tokens * 100`。
   - 分子应为最近一次模型调用的输入 token（`current_context_input_tokens`）。
3. **多跳工具轮次下的输入提取规则（ADR 0035 S1 冻结解析序）**：
   - `TurnStreamEvent` 携带 `provider_call_records: Option<Vec<ProviderCallCacheRecord>>`。
   - 若 `provider_call_records` 非空，取末条记录的 `input_tokens`（即本轮最终模型调用的真实 prompt 上下文）。
   - 若为空，则回退使用 `payload.input_tokens`。

## 1. 契约面定义（Protocol / TuiState）
### R1：`UiEvent::StreamUsage` 字段语义与扩展
```rust
UiEvent::StreamUsage {
    input_tokens: u64,           // 最近一次调用的上下文输入 token (S1 口径)
    output_tokens: u64,          // 本轮生成的输出 token
    cache_hit_input_tokens: u64, // 本轮缓存命中 token
    total_tokens: u64,           // 本轮总用量
}
```
### R2：`stream_usage_event` 提取逻辑
- 从 `TurnStreamEvent` 中提取 S1 当前上下文输入：
  ```rust
  let context_input = payload
      .provider_call_records
      .as_ref()
      .and_then(|records| records.last())
      .and_then(|rec| rec.input_tokens)
      .or(payload.input_tokens)
      .unwrap_or(0);
  ```
- `UiEvent::StreamUsage.input_tokens = context_input`。
- 输出、缓存与总量依旧透传。

### R3：`TuiState` 状态更新规则
- `current_context_input_tokens: u64`：记录当前上下文输入（替换旧的 total_input_tokens 作为 ↑ 和 上下文百分比分子的来源）。
- `total_output_tokens: u64`：保持会话累计输出。
- `total_cache_hit_tokens: u64`：保持会话累计缓存命中。
- `apply_turn_usage`：
  - `self.current_context_input_tokens = input;`（直接设置为最新单次调用的输入 token，不累加历史轮次）
  - `self.total_output_tokens` 与 `self.total_cache_hit_tokens` 继续保持累计（支持幂等覆盖去重）。
  - 上下文百分比计算：`current_context_input_tokens as f64 / context_window_tokens as f64 * 100.0`。

### R4：指标行渲染对齐
- `↑{in}`：渲染 `fmt_token_count(state.current_context_input_tokens)`。
- `上下文 {pct}%`：以 `state.current_context_input_tokens` 为分子计算比例。
- `缓存 {pct}%`：若 `state.current_context_input_tokens > 0`，比例为 `total_cache_hit_tokens / current_context_input_tokens`（或累计命中比）。

## 2. 验收测试契约
- A1：单轮单调用：S1 input 正确更新为该调用的 input_tokens，上下文百分比计算基于该 input。
- A2：多跳工具轮次：包含 2 次调用的 records（例如 1000 -> 1500），S1 input 正确提取最后一次调用的 1500，而非 2500。
- A3：跨轮次对话：第 1 轮 input 1000，第 2 轮 input 1200，S1 input 应为 1200，绝非 2200。
- A4：上下文百分比：分子为 1200 而非 1200+output。
- A5：旧测试套件兼容与适配。
