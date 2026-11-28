# Fact Sheet: Wave 4.2 (B075, B076, B078) 侦察清单

## 1. 技能生态健壮化与可观测性收口事实清单

- **B075 (技能边界覆盖)**: `render_prompt_prefix_with_budget` 的 `max_markers_budget` 为固定 512B，长描述场景下确实会压缩技能名显示，需逻辑优化。 — 证据: `crates/pony-agent-core/src/agent/workspace_hotloader.rs:1552`
- **B076 (/skill: 手势覆盖)**: `/skill:` 的处理逻辑位于 `bin/tui.rs` 中，但在 `queue::push_queued_message` 入队路径中未实现斜杠手势的识别逻辑，导致流式输入失效。 — 证据: `crates/pony-agent-core/src/bin/tui.rs:10166`
- **B078 (可观测性与 Schema 契约)**:
    - 缺错误码：`skill_not_available_for_model_invocation` 未在 `observability.yaml` 声明。 — 证据: `crates/pony-agent-core/src/agent/tools/orchestration.rs:792`
    - 缺 Schema 定义：`skill-manifest.v1.schema.json` 缺少 `disable-model-invocation` 字段定义。
    - 缺 name 校验：解析器在 name 缺失时直接回落目录名，缺乏 `omitted` 显式警告。

## 2. 覆盖盲区 (Coverage Gaps)
- 对 TUI 的 `/skill:` 流式队列入队路径无 E2E 测试。
- 缺少对「200+ 技能规模 + 长名称」的预算边界测试。
