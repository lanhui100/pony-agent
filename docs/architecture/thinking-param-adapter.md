# Thinking Parameter Adapter

思考模式/思考强度参数适配器架构。负责将 pony agent 统一的思考意图（开关、强度）映射到不同 provider/model 的特定参数格式。

## 设计目标

- 每添加一个新模型/提供商，只需新增一个 `ThinkingParamPattern` 变体 + 映射逻辑
- 不依赖 `is_deepseek_provider` 等脆弱命名检测
- 思考开关和强度在 pony agent 层面归一化，各 provider 自行适配

## 核心类型

`crates/pony-agent-core/src/agent/config.rs`

```rust
pub enum ThinkingParamPattern {
    /// 无思考参数
    None,
    /// reasoning_effort: low/medium/high/max (OpenAI 标准)
    EffortStandard,
    /// reasoning_effort: low/medium/high/none (DeepSeek V4, 火山引擎等)
    EffortWithNone,
    /// thinking: { type: "enabled/disabled" } (旧版 DeepSeek reasoner)
    ThinkingToggle,
    /// Anthropic thinking block with budget_tokens
    AnthropicThinking,
}
```

## 流程

```
ProviderCapabilityPreset
        │
        ▼
resolve_thinking_param_pattern()
        │
        ▼
ThinkingParamPattern  → 存储在 ResolvedProviderSelection.thinking_param_pattern
        │
        ▼
with_openai_request_options() / with_anthropic_request_options()
        │
        ▼
match pattern { ... }  → 写入 request body
```

## Preset → Pattern 映射

| ProviderCapabilityPreset | ThinkingParamPattern | 说明 |
|--------------------------|---------------------|------|
| `OpenAiChat` | `None` | 无思考 |
| `OpenAiReasoning` | `EffortStandard` | OpenAI o1/o3/GPT-5 系列 |
| `AnthropicThinking` | `AnthropicThinking` | Claude 思考模式 |
| `DeepseekChat` | `EffortWithNone` | DeepSeek V4 Flash |
| `DeepseekReasoner` + V4模型 | `EffortWithNone` | DeepSeek V4 Pro |
| `DeepseekReasoner` + 非V4 | `ThinkingToggle` | 旧版 deepseek-reasoner/r1 |
| `Auto` / `Custom`（DeepSeek + V4） | `EffortWithNone` | 自定义 DeepSeek V4 |
| `Auto` / `Custom`（DeepSeek + 非V4） | `ThinkingToggle` | 自定义旧版 DeepSeek |
| 其他 `Auto` / `Custom` | `None` | 默认无思考参数 |

## 各 Pattern 的请求参数行为

### `None`
不在 request body 中添加任何思考相关参数。

### `EffortStandard`
```json
// reasoning_effort 配置时：
{ "reasoning_effort": "low" | "medium" | "high" | "max" }

// 未配置时：不添加
```

归一化 → provider 值映射（`reasoning_effort_label`）：

| ProviderReasoningEffort | 值 |
|-------------------------|-----|
| `Low` | `"low"` |
| `Medium` | `"medium"` |
| `High` | `"high"` |
| `Max` | `"max"` |

### `EffortWithNone`
```json
// reasoning_effort 值：
{ "reasoning_effort": "low" | "medium" | "high" | "none" }

// 未配置时默认：{ "reasoning_effort": "medium" }
// "none" 表示关闭思考
```

归一化 → provider 值映射（`deepseek_reasoning_effort_label`）：

| ProviderReasoningEffort | 值 |
|-------------------------|-----|
| `Low` | `"low"` |
| `Medium` | `"medium"` |
| `High` | `"high"` |
| `Max` | `"high"`（DeepSeek 无 "max" 级别） |

### `ThinkingToggle`
```json
{ "thinking": { "type": "enabled" } }
```

注意：此模式下 `tool_choice` 会被移除（`thinking_param_supports_tool_choice()` 返回 `false`）。

### `AnthropicThinking`
```json
// reasoning_budget_tokens > 0 时：
{ "thinking": { "type": "enabled", "budget_tokens": N } }

// 否则不添加
```

## 如何添加新 Provider

以 Qwen（阿里云百炼）为例：

**Step 1**: 在 `ThinkingParamPattern` 添加变体

```rust
// config.rs
pub enum ThinkingParamPattern {
    // ... 已有变体
    QwenThinking,  // enable_thinking: bool + thinking_budget + preserve_thinking
}
```

**Step 2**: 在 `resolve_thinking_param_pattern` 添加映射

```rust
// config.rs
ProviderCapabilityPreset::Qwen => ThinkingParamPattern::QwenThinking,
```

（如果使用 `Auto` preset，通过 catalog 模式匹配自动关联）

**Step 3**: 在 `with_openai_request_options` 实现映射逻辑

```rust
// provider.rs
ThinkingParamPattern::QwenThinking => {
    let thinking_on = config.capabilities.supports_reasoning;
    body["enable_thinking"] = Value::Bool(thinking_on);
    if let Some(budget) = config.reasoning_budget_tokens {
        body["thinking_budget"] = json!(budget);
    }
    body["preserve_thinking"] = Value::Bool(true);
}
```

**Step 4**（可选）: 在 CAPABILITY_CATALOG 添加模式匹配

```rust
// config.rs
CapabilityCatalogEntry {
    protocol: Some("openai"),
    patterns: &["qwen"],
    preset: ProviderCapabilityPreset::Qwen,
},
```

## 关键代码位置

| 组件 | 文件 | 行 |
|------|------|-----|
| `ThinkingParamPattern` 枚举 | `config.rs` | — |
| `resolve_thinking_param_pattern()` | `config.rs` | `resolve_selection` 附近 |
| `is_deepseek_v4_by_model()` | `config.rs` | 同一文件 |
| `with_openai_request_options()` | `provider.rs` | dispatch logic |
| `thinking_param_needs_thinking_toggle()` | `provider.rs` | ThinkingToggle 专用 |
| `thinking_param_supports_tool_choice()` | `provider.rs` | 是否保留 tool_choice |
| `deepseek_reasoning_effort_label()` | `provider.rs` | EffortWithNone 映射函数 |
| `reasoning_effort_label()` | `provider.rs` | EffortStandard 映射函数 |
