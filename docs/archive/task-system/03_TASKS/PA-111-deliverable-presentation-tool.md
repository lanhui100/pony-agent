# PA-111: 结构化成果物声明工具 (present)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具契约
`present`
- 参数: `{ "files": Array<{ "path": string, "description"?: string }>, "cwd"?: string }`
- 返回: `{ "presented_files": Array<{ "path": string, "description": string, "size_bytes": number, "mime_type": string }> }`

### 验收矩阵
- **AC-01**: 将指定文件提升为终态 Deliverable 成果物并返回元数据（路径、描述、大小、MIME 类型）。
- **AC-02**: 必须断言文件真实存在且为普通文件（非目录、非设备），最多声明 4 个文件。
- **AC-03**: 严格工作区安全防护：拒绝声明工作区外的文件（Path Traversal Fail-Closed）。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与红相验证
- **负责人**: Team Lead
