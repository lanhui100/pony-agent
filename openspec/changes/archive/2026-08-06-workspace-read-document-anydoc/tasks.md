# Tasks

- [x] 研究 Firecrawl anydoc：能力边界、crates.io 可用性、与当前工具面的接入点。
- [x] `crates/pony-agent-core/Cargo.toml` 添加 `anydoc = "=0.1.6"` 并验证可编译。
- [x] 新建 `src/agent/document_conversion.rs`：`resolve_inside_workspace` 复用、输入/输出上限、`truncated` 证据、错误路径诊断。
- [x] `tools.rs` 注册 `workspace_read_document` descriptor（builtin_tools + 辅助映射函数）。
- [x] `governed_executor.rs` 注册 `ReadDocumentHandler`（仿 ViewImageHandler 分支）。
- [x] 单元测试：docx/csv 转换、内容级格式检测、越界拒绝、上限截断、缺参错误。
- [x] `npm run cargo:check:shared` 通过。
- [x] `npm run cargo:test:shared` 通过（core lib 全量 733 通过；tauri 回归 tool_router 13 + session 5 通过）。
- [x] 同步工具面文档（docs/architecture/tool-runtime-descriptor-registry.md 涉及处）。

## Validation Notes

- Passed: `npm run cargo:check:shared`（anydoc 0.1.6 编译通过，无新增警告）。
- Passed: `npm run cargo:test:exact -- --workspace --lib` — 733 passed（含 8 个新增 `document_conversion` 测试）。
- Passed: `npm run cargo:test:exact -- --workspace --test tool_router_regression` — 13 passed。
- Passed: `npm run cargo:test:exact -- --workspace --test session_regression` — 5 passed。
- 已更新工具面 characterization 基线：`tools.rs` 三个 contract_view 测试 + `provider/mod.rs` 两个 payload 测试（新增 ReadDocument 产品名）。
- 预存失败（与本次改动无关，未处理）：`provider_registry_regression::resolve_selection_falls_back_to_selected_provider_and_model` 断言 `max_output_tokens == 64000`（config.rs 内置默认 `DEFAULT_MODERN_MAX_OUTPUT_TOKENS`），实际 resolve 返回所选模型配置值 4096 —— `a2636da`（Fix provider model selection persistence and dedupe）后回归测试期望未同步。
- 已知 flaky（预存，与本次改动无关）：`agent::session::tests::cleanup_attachment_assets_only_reclaims_unreferenced_payloads` 全量并发偶发失败、单独跑稳定通过（时间窗口竞争）。
