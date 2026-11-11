# PA-105: 异步后台作业生命周期工具族 (job_*)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具清单
1. `job_start`
   - 参数: `{ "command": string, "args"?: string[], "cwd"?: string, "timeout_ms"?: number }`
   - 返回: `{ "job_id": string, "pid": number, "status": "running" }`
2. `job_output`
   - 参数: `{ "job_id": string, "wait"?: boolean, "timeout_ms"?: number, "offset"?: number }`
   - 返回: `{ "job_id": string, "status": "running" | "completed" | "failed" | "killed", "output": string, "cursor": number, "exit_code"?: number }`
3. `job_kill`
   - 参数: `{ "job_id": string, "reason"?: string }`
   - 返回: `{ "job_id": string, "killed": boolean }`
4. `job_list`
   - 参数: `{}`
   - 返回: `{ "jobs": Array<{ "job_id": string, "command": string, "status": string, "pid": number, "started_at": u64 }> }`

### 验收矩阵
- **AC-01**: `job_start` 异步启动进程，立即返回 Job ID 与 PID，主线程不阻塞。
- **AC-02**: `job_output` 支持基于 `offset` 的单调递增游标拉取与 `wait` 阻塞等待完成/超时。
- **AC-03**: `job_kill` 强杀子进程树并回收资源，状态变更为 `killed`。
- **AC-04**: `job_list` 列举所有活跃与历史（定长保留）作业。
- **AC-05**: 路径穿越校验与沙箱 Fail-Closed。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与红相验证
- **负责人**: Team Lead
