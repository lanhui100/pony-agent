# Stage 1: Provider Temperature 处理契约矩阵

## 1. 核心问题与背景
在 OpenAI 协议（Chat Completions）请求构建中，当前代码在 `decision` 与 `followup`（包括 stream 和 sync）四处硬编码了 `"temperature": request.temperature`。
对于推理模型（如 `kimi-k3`、OpenAI `o1`/`o3`、`deepseek-reasoner` 等）或上游网关严格校验的模型，传入温度（尤其是默认的 `0` 或 `0.2`）会导致 HTTP 400 报错（如 `field Temperature invalid`）。
而在 `responses_api.rs` 中，已经实现了当 `capabilities.supports_reasoning == true` 时省略 `temperature` 字段的策略。

## 2. 行为契约
1. **统一通过 `with_openai_request_options` 或辅助函数控制 `temperature`**：
   - 规则：当模型为 reasoning 模型时（即 `config.capabilities.supports_reasoning == true`），从请求 body 中**移除或不附加** `temperature` 字段。
   - 对非 reasoning 模型：保留 `temperature` 字段。
2. **影响的调用点**：
   - `crates/pony-agent-core/src/agent/provider/mod.rs`：
     - `send_openai_tool_followup_request_sync` (L852)
     - `send_openai_tool_followup_request_stream` (L933)
     - `send_openai_decision_request_sync` (L1532)
     - `send_openai_decision_request_stream` (L1606)
   - 在这四处调用 `with_openai_request_options` 时，统一执行该契约规则。
3. **兼容性与不变量**：
   - 非 reasoning 模型的原有 temperature 行为完全不变。
   - reasoning 模型的请求 payload 中坚决不出现 `"temperature"` key。
