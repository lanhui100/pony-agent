# fix-provider-registry-fallback-test-baseline

## Why

`src-tauri/tests/provider_registry_regression.rs` 的回归测试
`resolve_selection_falls_back_to_selected_provider_and_model`（第 384 行）当前失败：

- 测试先保存 `beta-provider`，其包含两个模型 `beta-chat`（`max_output_tokens=4096`）与
  `beta-fallback`（`max_output_tokens=8192`），`selected_model_id = Some("beta-fallback")`。
- 随后调用 `store.resolve_selection(Some("missing"), Some("missing"))`，断言
  `resolved.max_output_tokens == 64000`，但实际解析结果为 `4096`。

期望值 64000（即 `DEFAULT_MODERN_MAX_OUTPUT_TOKENS`）是在提交 `a2636da`
（Fix provider model selection persistence and dedupe）之前有效的基线：当时 provider 内不存在
模型值去重，`beta-fallback` 的 8192 会在 `normalize_storage` 中升级为 64000，`resolve_selection`
按 `selected_model_id` 回退后恰好命中该模型。`a2636da` 引入的 `dedupe_provider_models`
会按模型值（`model_uniqueness_key`）去重：`beta-chat` 与 `beta-fallback` 共享同一个
`model` 值 `claude-3-7-sonnet-latest`，后者被去重删除，`selected_model_id` 被重定向到
保留项 `beta-chat`。因此当前实现解析出的 `max_output_tokens` 是 4096，符合 `a2636da`
之后的设计意图——`resolve_selection` 的 fallback 语义（provider 回退到
`selected_provider_id`、model 回退到 `selected_model_id`）从未改变，改变的是保存路径上
对重复模型值的去重规则。

当前实现行为是正确的（去重行为有 `a2636da` 自带的单测
`normalize_storage_deduplicates_model_values_within_provider` 锁定）；测试期望值 64000
是过时基线，需要更新。

## What Changes

- 更新 `src-tauri/tests/provider_registry_regression.rs` 第 384 行断言，将
  `resolved.max_output_tokens` 期望值从 `64000` 改为 `4096`，并补充注释说明依据：
  `a2636da` 的模型去重规则使 `beta-fallback`（与 `beta-chat` 共享模型值）被删除、
  `selected_model_id` 重定向到 `beta-chat`，故 fallback 解析结果为 `beta-chat`
  的 `max_output_tokens=4096`。
- 不修改 `resolve_selection` 及任何生产代码（当前行为与 `a2636da` 意图一致）。
- 建立本 OpenSpec 变更（proposal/design/tasks/specs）并归档测试基线的根因结论。
- 在 `docs/INDEX.md` 8.1 小节登记本变更。

## Background

- `resolve_selection`（`crates/pony-agent-core/src/agent/config.rs`）的 fallback 顺序为：
  传入的 `provider_id` → `selected_provider_id` → 第一个 provider；模型同理：
  传入的 `model_id` → `selected_model_id` → 第一个模型。该语义自 `dd3dd8e`
  （引入该测试）起未变。
- `a2636da` 在 `normalize_storage` 中新增 `dedupe_provider_models`：按模型值
  （`model_uniqueness_key`）保留首个模型，删除后续重复项，并把被删除项上的
  `selected_model_id` 重定向到保留项。
- 测试中 `beta-chat` 与 `beta-fallback` 的 `model` 值均为
  `claude-3-7-sonnet-latest`，在去重后只剩 `beta-chat`（4096）。

## Goals

- 让 `provider_registry_regression` 回归测试全量通过。
- 使测试断言反映当前（且被生产代码单测锁定的）去重 + fallback 语义，而不是
  `a2636da` 之前的过时基线。
- 为后续维护者留下根因说明，避免再次误判为生产逻辑 bug。

## Non-goals

- 不修改 `resolve_selection` / `normalize_storage` / `dedupe_provider_models` 的生产行为。
- 不改变 provider 模型去重的产品语义（保留首个、selected 重定向）。
- 不引入"fallback 到内置默认 modern 模型（64000）"的新语义——当前设计意图是
  fallback 到用户已选/持久化的 provider 与模型。

## Scope

- `src-tauri/tests/provider_registry_regression.rs` 单行断言更新 + 注释。
- OpenSpec 变更目录与 `docs/INDEX.md` 文档登记。
- 不涉及 core crate 生产代码，因此不触碰 `crates/pony-agent-core` 单测。

## Risks

- 若未来再次改动模型去重或 fallback 规则，本测试基线需要随之复查；注释中已记录
  `a2636da` 行为锚点便于追溯。
- 其余对 `64000` 的断言（第 128 行、`config.rs` 1584 行）仍基于"legacy 默认值
  （1200/8192）升级为 64000"的规则，不受本变更影响，无需改动。

## Validation

- `npm run cargo:check:shared`
- `npm run cargo:test:exact -- --workspace --test provider_registry_regression`
- 验证结果写入 `tasks.md` 的 Validation Notes。
