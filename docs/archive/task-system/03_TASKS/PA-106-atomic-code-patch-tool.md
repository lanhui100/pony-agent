# PA-106: 原子代码补丁应用工具 (apply_patch)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具契约
`apply_patch`
- 参数: `{ "patch": string }`（标准 Unified Diff 格式，支持单文件与多文件，含 `--- a/path` 与 `+++ b/path`）
- 返回: `{ "success": boolean, "files_modified": string[], "files_created": string[], "files_deleted": string[] }`

### 验收矩阵
- **AC-01**: 正确解析与应用单文件与多文件 Unified Diff 补丁（修改、新增、删除）。
- **AC-02**: 全有或全无原子性保证（All-or-Nothing）：若多文件补丁中任一 hunk 或文件校验失败，所有修改立即回滚，零脏写。
- **AC-03**: 路径逃逸防护：补丁内包含 `../` 或指向工作区外的路径必须抛出安全拒绝错误并阻止所有写入。
- **AC-04**: CRLF 与 LF 换行符容错，并保持原文件换行风格。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与红相验证
- **负责人**: Team Lead
