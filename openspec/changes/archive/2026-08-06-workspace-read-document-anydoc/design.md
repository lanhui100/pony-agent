# Design

## Decision Summary

新增一个 workspace-scoped 的文档转换内置工具 `workspace_read_document`：

1. 转换内核直接复用 Firecrawl `anydoc` crate（crates.io `anydoc = "=0.1.6"`），纯本地、毫秒级、无外部服务。
2. 安全边界完全复用 `view_image`（`image_artifact.rs`）的既有模式：`resolve_inside_workspace` 路径约束、显式上限、`truncated` 证据。
3. 注册为 ModelVisible 内置工具，通过 `build_governed_executor` 挂专用 `PrimitiveToolHandler`（仿 `ViewImageHandler`），不经过 legacy `ToolRouter` 的字符串分发。

## Chosen Direction

### 1. 新模块 `document_conversion.rs`

仿 `image_artifact.rs` 的模块结构，保持"模块回答它负责什么、不负责什么"的项目惯例：

- `DocumentReadOptions`：`max_output_bytes`（输出 Markdown 上限，默认 512 KiB）、`max_input_bytes`（输入文件上限，默认 20 MiB）。
- `read_workspace_document(path, root, options) -> Result<DocumentConversion, String>`：
  - 空路径 / 非文件 / 越界路径 → fail-closed 错误（复用 `resolve_inside_workspace`）。
  - 输入超过 `max_input_bytes` → 明确拒绝。
  - `anydoc::to_markdown_bytes(&bytes, None)` 内容级格式检测；失败时附加 `Format::from_path` 诊断信息，明确区分"不支持格式"与"解析失败"。
  - 输出超过 `max_output_bytes` → 截断并在结果中给出 `truncated: true` 与完整长度，绝不静默吞掉。
- `DocumentConversion`：`path`（canonical 绝对路径）、`format`（anydoc 检测出的 `Format`）、`markdown`、`markdown_len`（完整长度）、`truncated`、`input_bytes`。

### 2. 工具 descriptor

在 `tools.rs` 的 `builtin_tools()` 注册：

- 名称：`workspace_read_document`；描述说明支持格式与限制（不含 OCR）。
- input schema：`{ path: string, maxOutputBytes?: integer }`，`additionalProperties: false`。
- 辅助映射函数 `tool_kind_for_name` / `default_permission_declaration_for_name` / `execution_policy_for_primitive` / `tool_display_metadata_for_name` / `tool_exposure_for_name` 若对未知 primitive 有默认兜底则走默认（读权限声明、workspace.read 权限域），并补充 display metadata 显示名"读取文档（Office/PDF → Markdown）"。

### 3. governed executor 注册

`build_governed_executor` 中仿 `TOOL_VIEW_IMAGE` 分支：

```rust
if primitive == TOOL_READ_DOCUMENT {
    dispatcher.register_handler(
        descriptor.identity.descriptor_id.clone(),
        Arc::new(ReadDocumentHandler::new(workspace.clone())) as Arc<dyn PrimitiveToolHandler>,
    );
    continue;
}
```

`ReadDocumentHandler` 实现 `PrimitiveToolHandler::execute`，解析 `path` 参数（缺省报错），调用 `read_workspace_document`，返回 `ToolOutcome` 兼容的 JSON（`path` / `format` / `markdown` / `truncated` 字段）。

### 4. 输出格式

handler 返回结构化 JSON 而非裸文本，便于前端与日志展示格式信息：

```json
{
  "path": "<canonical absolute path>",
  "format": "docx",
  "markdown": "...",
  "truncated": false
}
```

## Rejected Alternatives

### legacy ToolRouter 内实现（`tools.rs` 的 `match primitive` 分发）

拒绝理由：PA-076 后 governed dispatcher 已是默认执行引擎，`RouterPrimitiveHandler` 只是兼容转发层；新工具走专用 handler 可获得与 `view_image` / `plan_control` 一致的一等公民治理路径，避免双路径维护。

### 接入 Firecrawl 托管 Parse API（`firecrawl` crate）

拒绝理由：引入 API key、网络依赖与按页计费，违反本工具"离线、本地、无外部服务"的目标；且扫描版 OCR 不在本次需求内。

### Node 侧 `@firecrawl/anydoc` 绑定

拒绝理由：工具执行属于 Rust 核心，经 Tauri command 绕行前端再转换徒增跨语言桥接与二进制分发复杂度。

### Agent Skill 方式（CLI 子进程）

拒绝理由：绕过工具治理（权限声明、预算、可观测性、沙箱裁决），与项目"统一抽象优先于临时堆叠"的原则冲突。

## Risks and Mitigations

### anydoc 0.1.x 行为不稳定

锁定 `=0.1.6`；`document_conversion` 只依赖 `to_markdown_bytes` + `Format` 两个稳定 API，升级时 diff 收敛面小。

### 超长文档输出撑爆上下文

`max_output_bytes` 截断 + `truncated` 证据 + `max_input_bytes` 输入闸门，三者叠加保证模型看到的输出有界且诚实。

### 编译依赖增加

anydoc 为纯 Rust（zip/OLE/PDF 解析），无系统库；首次编译时间成本一次性接受，产物不增加运行时外部进程。

### 扫描版 PDF 无法转换

anydoc 无 OCR，扫描件解析会失败或产出残缺 Markdown。错误路径返回清晰提示（"scanned document requires OCR, not supported locally"），文档中记录适用边界。

## Review Record

- 方案经用户确认：Rust 原生内置工具（推荐项）、走 OpenSpec 变更、命名 `workspace_read_document` 且 ModelVisible 暴露。
