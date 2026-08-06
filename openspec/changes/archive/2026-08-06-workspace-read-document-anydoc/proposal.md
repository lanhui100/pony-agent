# workspace-read-document-anydoc

## Background

当前工作区工具面只能处理文本类文件：`workspace_read_file` / `workspace_read_file_segment` 直接按行读取，`view_image` 只负责图片。办公文档（Word / PowerPoint / Excel / PDF / ODF / RTF / EPUB / CSV）在对话中完全不可读——模型既不能理解 `.docx` 里的内容，也不能把 `.pdf` 报告转为可分析的上下文。

Firecrawl 于 2026-08-03 开源了 `anydoc`（MIT，crates.io `anydoc 0.1.6`）：纯 Rust 的文档 → GitHub Flavored Markdown 转换引擎，中位转换 <5ms，支持 14 种格式，内容级格式检测（读字节魔数而非扩展名），无需 ML 模型与外部服务。它是 Firecrawl 托管 `/parse` API 的底层引擎，质量经 100 份真实文档的盲测（benchmark score 80，领先 docling/libreoffice/unstructured）。

## Goals

- 新增内置工具 `workspace_read_document`：把工作区内受支持的办公文档转换为 Markdown 文本返回给模型。
- 沿用 `view_image` 的成熟安全模式：路径必须解析在工作区内、输出带上限、截断以显式 `truncated` 证据呈现。
- 工具通过 governed executor 注册为 ModelVisible，进入统一的权限/预算/可观测性治理链路。
- 全离线本地转换，无任何外部服务调用。

## Non-goals

- 不接入 Firecrawl 托管 API：不提供 OCR（扫描版 PDF 明确报错并说明原因）。
- 不传输/渲染文档内嵌图片的二进制内容（嵌入资源以 alt 文本呈现于 Markdown，与 anydoc 输出一致）。
- 不新增前端专用 UI，不修改 provider 协议与消息格式。
- 不替换 `workspace_read_file` 的文本路径。

## Scope

- `crates/pony-agent-core`：新增 `anydoc` 依赖、新模块 `document_conversion`、builtin descriptor 注册、governed executor 专用 handler、单元测试。
- 文档：`docs/INDEX.md` 与 `docs/architecture/tool-runtime-descriptor-registry.md` 涉及工具面的部分同步。

## Risks

- `anydoc` 处于 0.1.x 快速迭代期：输出格式与行为可能随版本变化。缓解：锁定版本 `=0.1.6`，转换函数收敛为薄封装，便于升级审计。
- 新依赖引入约数十个传递 crate（zip、OLE 解析等），增加首次编译时间。缓解：仅在 core crate 添加，不在 tauri 侧重复依赖。
- 扫描版 PDF / 复杂排版文档转换质量有限。缓解：错误信息明确说明限制，文档中记录适用边界。
- 超长文档输出可能撑爆上下文。缓解：输出字节上限 + `truncated` 证据 + 输入文件大小上限。

## Validation

- 单元测试覆盖：docx/csv 真实转换、内容级格式检测、越界路径拒绝、非文档文件拒绝、输出超限截断证据。
- `npm run cargo:check:shared` 通过。
- `npm run cargo:test:shared` 通过（含新增测试）。
