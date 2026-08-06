# 2026-08-06 workspace_read_document（Firecrawl anydoc）收口

## 本轮目标

- 将 Firecrawl 开源文档转换引擎 `anydoc` 集成为 Pony Agent 内置工具 `workspace_read_document`，并完成 OpenSpec 变更归档与文档同步。
- 全程仅触碰 Rust core 工具面与文档，未改动前端与 provider 协议。

## 完成事实（与代码核对）

- **依赖**：`crates/pony-agent-core/Cargo.toml` 添加 `anydoc = "=0.1.6"`（MIT、纯 Rust、rust-version 1.88；本机 1.95 满足）；dev-dependencies 添加 `zip = "8.6"` 用于测试 fixture 构造。
- **新模块** `crates/pony-agent-core/src/agent/document_conversion.rs`：
  - `read_workspace_document(path, root, options)`：workspace canonical 路径约束（fail closed）、输入上限 20 MiB、输出上限 512 KiB + `truncated` 证据 + 完整 `markdown_len`。
  - 格式检测：`Format::from_bytes`（内容级）优先，`from_path`（扩展名）回退——CSV 无字节签名必须命名。
  - 错误路径区分"不支持内容"与"解析失败"，扫描版 PDF 明确提示需 OCR。
  - `ReadDocumentHandler`（`PrimitiveToolHandler`），与 `ViewImageHandler` 同模式。
- **注册**：`tools.rs` 注册 `workspace_read_document` descriptor（ModelVisible、`workspace.read`、ToolKind::Read、别名 `workspace.read_document`、产品名 `ReadDocument`、显示名"读文档"、`concurrent_safe` + 512 KiB result budget）；`governed_executor.rs` 注册专用 handler 并加入只读子调用白名单（governed `READ_ONLY_PRIMITIVES` 与 legacy `READ_ONLY_BATCH_CHILD_PRIMITIVES` 同步）。
- **测试**：8 个新单元测试（docx 内容级检测含 mislabel、csv 扩展名回退、越界/缺失/目录拒绝、输入超限、输出截断证据、不支持内容诊断、handler 参数解析）；更新工具面 characterization 基线（tools.rs 3 处 + provider/mod.rs 2 处，新增 `ReadDocument` 产品名，已有工具相对顺序不变）。

## 最近一次验证快照

- `npm run cargo:check:shared`：通过，无新增警告。
- `npm run cargo:test:exact -- --workspace --lib`：**733 passed**（含 8 个新测试）。
- `npm run cargo:test:exact -- --workspace --test tool_router_regression`：13 passed。
- `npm run cargo:test:exact -- --workspace --test session_regression`：5 passed。

## 预存问题（与本次改动无关，未处理）

1. `src-tauri/tests/provider_registry_regression.rs::resolve_selection_falls_back_to_selected_provider_and_model` 失败：断言 `max_output_tokens == 64000`（`config.rs` 内置默认 `DEFAULT_MODERN_MAX_OUTPUT_TOKENS`），实际 resolve 返回所选模型配置值 4096。`a2636da`（Fix provider model selection persistence and dedupe）后回归测试期望未同步。→ 已委派子智能体建立修复变更（见下一步动作 2）。
2. `agent::session::tests::cleanup_attachment_assets_only_reclaims_unreferenced_payloads`：全量并发偶发失败、单独跑稳定通过（时间窗口竞争，预存 flaky）。

## 归档与文档同步

- OpenSpec 变更 `workspace-read-document-anydoc` 已归档：`openspec/changes/archive/2026-08-06-workspace-read-document-anydoc/`。
- canonical spec 已生成并补齐 Purpose：`openspec/specs/workspace-read-document/spec.md`（4 条 requirement）。
- `docs/architecture/tool-runtime-descriptor-registry.md`：新增 "Workspace Document Conversion" 段落 + Code Layout 表行。
- `docs/INDEX.md`：新增 8.1 工具面新增变更小节，指向归档变更。

## 下一步动作

1. **提交**：仅提交本任务文件（core Cargo.toml / Cargo.lock / document_conversion.rs / tools.rs / governed_executor.rs / mod.rs / provider/mod.rs / 文档 / 归档变更目录），不含其他会话遗留改动（control_plane/mod.rs、runtime/mod.rs、src-tauri 下的临时 py 文件等）。
2. **provider 回归测试修复变更**：已委派子智能体（backend-dev）建立 OpenSpec 变更并修复 `resolve_selection_falls_back_to_selected_provider_and_model` 期望（确认 `a2636da` 变更意图后决定是修逻辑还是修测试基线）。

## 断点续跑提示

- 编译验证一律走 npm script 槽位（`cargo:check:shared` / `cargo:test:exact -- --workspace ...`），不要直接跑 cargo。
- 新工具行为验证：`workspace_read_document`（path 相对工作区；返回 format/markdown/truncated）。
- provider 修复变更工作区位于 `openspec/changes/`（子智能体创建后），与本次归档变更互不依赖。
