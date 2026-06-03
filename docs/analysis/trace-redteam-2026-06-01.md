# Trace TTFT / token/s 红队校验记录（2026-06-01）

## 目标

- 校验 trace 面板中的 `首 token 延时` 是否按“从 turn 开始到首次收到 provider 可见增量/响应”计算
- 校验 trace 面板中的 `token/s` 是否与真实事件时间一致
- 校验真实链路下多次 `call model` / `call tool` 是否影响以上指标口径

## 本轮修正

### 运行时口径修正

文件：
- [runtime.rs](C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)

实际改动：
- 对“同步拿完整响应，再本地模拟 `turn:delta`”路径，`first_token_latency_ms` 不再使用同步请求返回耗时
- 改为按第一次真实发出可见 `turn:delta` 的时刻计算

### 红队探针修正

文件：
- [trace_redteam_probe.rs](C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/bin/trace_redteam_probe.rs)

实际改动：
- 使用 `start_turn_stream` 直接监听 `turn:delta / turn:completed / turn:failed`
- 同时输出 `reported_ttft_ms` 与 `observed_ttft_ms`
- 同时输出 `reported_token_per_second` 与 `observed_token_per_second`
- 每次 probe 使用独立 `session_id`，避免历史上下文污染样本
- 修正 probe 起点，避免把 `AgentRuntime::new()` 初始化时间误算进观测 TTFT
- 输出前 8 条事件明细，方便定位误差来源

## 校验命令

无工具基线：

```powershell
cargo run --bin trace_redteam_probe -- --provider provider-ppx --prompt "不要调用任何工具，只输出OK。"
cargo run --bin trace_redteam_probe -- --provider provider-deepseek --prompt "不要调用任何工具，只输出OK。"
```

工具 hop 样本：

```powershell
cargo run --bin trace_redteam_probe -- --provider provider-ppx --prompt "请先读取当前工作区的 Cargo.toml，确认 package.name，然后只用一句中文回答。"
cargo run --bin trace_redteam_probe -- --provider provider-deepseek --prompt "请先读取当前工作区的 Cargo.toml，确认 package.name，然后只用一句中文回答。"
```

相关 Rust 校验：

```powershell
cargo test start_turn_stream_emits_first_token_latency_on_reasoning_delta -- --nocapture
cargo test start_turn_stream_measures_first_token_latency_from_turn_start_across_tool_hops -- --nocapture
```

## 结果

### 无工具基线

#### ppx / gpt-5.4-mini

- `reported_ttft_ms=4891`
- `observed_ttft_ms=4891`
- `ttft_bias_ms=0`
- `reported_token_per_second=1.15`
- `observed_token_per_second=1.15`
- `token_per_second_bias_pct=0.00`

结论：
- TTFT 与 token/s 已完全对齐

#### deepseek / deepseek-v4-flash

- `reported_ttft_ms=3221`
- `observed_ttft_ms=3221`
- `ttft_bias_ms=0`
- `reported_token_per_second=4.29`
- `observed_token_per_second=4.29`
- `token_per_second_bias_pct=0.00`

结论：
- TTFT 与 token/s 已完全对齐

### 工具 hop 样本

#### ppx / gpt-5.4-mini

- `reported_ttft_ms=5455`
- `observed_ttft_ms=5457`
- `ttft_bias_ms=2`
- `reported_token_per_second=2.82`
- `observed_token_per_second=2.82`
- `token_per_second_bias_pct=-0.05`

链路摘要：
- `turn:started`
- `turn:trace (calling_tool)`
- `turn:tool`
- `turn:tool`
- `turn:trace (calling_model)`
- 多次 `turn:delta`
- `turn:completed`

结论：
- 多 hop 链路下口径仍保持一致

#### deepseek / deepseek-v4-flash

- `reported_ttft_ms=3829`
- `observed_ttft_ms=3829`
- `ttft_bias_ms=0`
- `reported_token_per_second=143.60`
- `observed_token_per_second=143.60`
- `token_per_second_bias_pct=0.00`

链路摘要：
- `turn:started`
- `turn:trace (calling_tool)`
- `turn:tool`
- `turn:tool`
- `turn:trace (calling_model)`
- `turn:delta`
- `turn:completed`

结论：
- 多 hop 链路下口径仍保持一致

## 额外发现

`deepseek` 的工具 follow-up 流式路径存在单独兼容性问题：

- `provider_followup_stream` 返回 `400 Bad Request`
- 错误信息：`The reasoning_content in the thinking mode must be passed back to the API.`
- 当前会退化到 fallback 路径后继续完成 turn

这不会影响本次 TTFT / token/s 口径结论，但它是独立的 provider 兼容性问题，建议单独修复。

## 当前结论

- trace 面板里的 `首 token 延时` 口径已经和真实流事件对齐
- trace 面板里的 `token/s` 口径已经和真实流事件对齐
- 无工具路径与多 hop 工具路径都已验证
- 目前没有发现“当前 pony agent 在已验证 provider/model 上存在较大偏差”

## 尚未覆盖

以下 provider 当前机器没有可用 key，因此本轮无法产出有效样本：

- `openai`
- `openrouter`
- `anthropic`

后续只要补齐 key，可继续复用同一 probe 命令直接回归。
