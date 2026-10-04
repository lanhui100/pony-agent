# PA-107: 语言服务器协议（LSP）智能工具 (lsp_*)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具清单
1. `lsp_definition`
   - 参数: `{ "file_path": string, "line": number, "character": number }`
   - 返回: `{ "locations": Array<{ "uri": string, "range": { "start": { "line": number, "character": number }, "end": { "line": number, "character": number } } }> }`
2. `lsp_references`
   - 参数: `{ "file_path": string, "line": number, "character": number, "include_declaration"?: boolean }`
   - 返回: `{ "locations": Array<{ "uri": string, "range": { "start": { "line": number, "character": number }, "end": { "line": number, "character": number } } }> }`
3. `lsp_hover`
   - 参数: `{ "file_path": string, "line": number, "character": number }`
   - 返回: `{ "contents": string }`
4. `lsp_diagnostics`
   - 参数: `{ "file_path": string }`
   - 返回: `{ "diagnostics": Array<{ "severity": string, "message": string, "range": { "start": { "line": number, "character": number }, "end": { "line": number, "character": number } } }> }`

### 验收矩阵
- **AC-01**: 支持常用编程语言的代码符号跳转定义（Definition）与查询引用（References）。
- **AC-02**: 支持悬停信息（Hover）与文件级/工作区语法语义诊断（Diagnostics）。
- **AC-03**: 当对应语言服务器未安装或不可用时，具备优雅降级（Graceful Degradation）与 Fail-Safe 机制，不导致 Agent 崩溃。
- **AC-04**: 工作区安全边界校验：拒绝查询工作区外未授权文件。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与红相验证
- **负责人**: Team Lead
