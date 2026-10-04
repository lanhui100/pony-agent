# PA-108: 极速模糊文件检索与 Git 差异工具 (fuzzy_file_search / git_diff_remote)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具清单
1. `fuzzy_file_search`
   - 参数: `{ "query": string, "max_results"?: number, "cwd"?: string }`
   - 返回: `{ "matches": Array<{ "path": string, "score": number }> }`
2. `git_diff_remote`
   - 参数: `{ "target_ref"?: string, "cwd"?: string }`
   - 返回: `{ "diff": string, "files_changed": string[], "has_remote": boolean }`

### 验收矩阵
- **AC-01**: `fuzzy_file_search` 提供工作区内快速路径模糊打分检索，支持自定义最大返回数，自动忽略 `.git` 等常见构建目录。
- **AC-02**: `git_diff_remote` 提取本地与远端/上游分支的差异 diff 及受影响文件清单。
- **AC-03**: 安全边界校验：拒绝通过参数注入执行非预期命令（命令参数数组安全传递），杜绝路径逃逸。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与红相验证
- **负责人**: Team Lead
