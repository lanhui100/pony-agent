# Design

## Root Cause Analysis

### 失败链

1. 测试保存 `beta-provider`：两个模型 `beta-chat`（4096）与 `beta-fallback`（8192），
   `selected_model_id = "beta-fallback"`。
2. `save_view` → `normalize_storage`（`crates/pony-agent-core/src/agent/config.rs:709`）：
   - 模型循环（772-810 行）先执行 `max_output_tokens` 升级：`0 / 1200 / 8192`
     会被提升为 `DEFAULT_MODERN_MAX_OUTPUT_TOKENS`（64000）。`beta-chat`（4096）
     不受影响；`beta-fallback`（8192）本会被升级为 64000。
   - 随后 `dedupe_provider_models(provider)`（827 行，`a2636da` 引入）按模型值去重。
     `beta-chat` 与 `beta-fallback` 的 `model` 值都是 `claude-3-7-sonnet-latest`，
     二者 `model_uniqueness_key` 相同；`beta-fallback` 作为重复项被删除，
     `selected_model_id` 从 `"beta-fallback"` 重定向到保留项 `"beta-chat"`。
3. 持久化后 beta-provider 仅剩 `beta-chat`（4096），`selected_model_id = "beta-chat"`。
4. `resolve_selection(Some("missing"), Some("missing"))`：provider 回退到
   `selected_provider_id`（beta），模型回退到 `selected_model_id`（`beta-chat`）
   → `max_output_tokens = 4096`。

### 语义对照

| 维度 | 结论 |
|---|---|
| `resolve_selection` fallback 语义 | 未变（`dd3dd8e` 起即：传入 id → selected → first），当前实现符合设计 |
| `a2636da` 意图 | 修复模型选择持久化 + 模型值去重；`dedupe_provider_models` 保留首个重复项并把 selected 重定向到保留项，有单测 `normalize_storage_deduplicates_model_values_within_provider` 锁定 |
| 测试期望 64000 | 基于 `a2636da` 之前的基线（无去重，`beta-fallback` 8192→64000 后命中）；该路径已不可达，期望过时 |

### 其他 64000 断言评估

- `provider_registry_regression.rs:128`：模型 `gpt-5-alpha`（1200）→ 升级 64000，
  单模型无去重影响，仍正确，不改。
- `config.rs:1584`（core 单测）：单模型 8192 → 升级 64000，仍正确，不改。

## Decision Summary

**修复方向：更新测试基线（option b）。**

- 将 `provider_registry_regression.rs:384` 断言从 `64000` 改为 `4096`，并加注释说明
  `a2636da` 去重规则的影响。
- 生产代码零改动：`resolve_selection` 当前行为与 `a2636da` 意图一致，改实现会破坏
  `normalize_storage_deduplicates_model_values_within_provider` 单测与去重语义。

测试名称 `resolve_selection_falls_back_to_selected_provider_and_model` 仍然准确：
它验证的正是"传入不存在的 id 时回退到持久化的 selected provider + selected model"，
4096 正是被去重重定向后的 selected model（`beta-chat`）的真实值。

## Rejected Alternatives

### 修改 `dedupe_provider_models` 优先保留 selected 的重复模型

拒绝。`a2636da` 的单测
`normalize_storage_deduplicates_model_values_within_provider` 明确断言"保留首个、
selected 重定向到保留项"（`model-gpt-5-copy` 被删、selected 改为 `model-gpt-5`）。
改去重优先级将违反该提交的既定意图并破坏其单测，属于超出本回归修复范围的行为变更。

### 让 `resolve_selection` 在 id 全部缺失时回退到内置默认 modern 模型（64000）

拒绝。引入"内置默认模型兜底"会覆盖用户持久化的 provider/model 选择，改变产品语义；
且当前 fallback 到 selected 的行为符合 `a2636da` 及更早 `dd3dd8e` 的设计。

### 将测试中两个模型的 `model` 值改为不同值以绕过去重

拒绝。虽然可以恢复"命中 `beta-fallback`"的路径，但会削弱回归测试对去重 + fallback
组合行为的覆盖，且使测试数据偏离真实场景（同一 provider 内同模型值不同配置的模型
是实际存在的用户数据形态）。

## Risks and Mitigations

### 未来去重/fallback 规则再次变化导致基线失真

Mitigation：断言注释记录 `a2636da` 行为锚点与两条独立的期望值来源
（升级规则 64000 vs 去重后保留值 4096），便于后续复查。

### 维护者误将 4096 当作"未升级的旧值"

Mitigation：注释明确说明 `beta-chat` 的 4096 既不在升级规则（0/1200/8192）之内、
也是去重后的保留项，二者共同决定该值，避免误改回 8192/64000。

## Review Record

- Root cause 调查结论：`a2636da` 未改动 `resolve_selection` 本体，但新增的
  `dedupe_provider_models` 使测试构造的重复模型被去重，从而令 64000 基线不可达。
- 方向裁决：当前实现正确、测试期望过时 → 更新测试基线，不触碰生产逻辑。
