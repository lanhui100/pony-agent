# PA-110: 智能体派生与协作编排工具族 (subagent / workflow / teams)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具清单
1. `subagent`
   - 参数: `{ "prompt": string, "description": string, "run_in_background"?: boolean }`
   - 返回: `{ "subagent_id": string, "status": "completed" | "running", "output"?: string }`
2. `workflow`
   - 参数: `{ "name": string, "steps": Array<{ "id": string, "action": string, "depends_on"?: string[] }> }`
   - 返回: `{ "workflow_id": string, "executed_steps": string[], "status": "success" | "failed" }`
3. `team_task_create`
   - 参数: `{ "subject": string, "description": string, "write_scopes"?: string[], "blocked_by"?: string[] }`
   - 返回: `{ "task_id": string, "revision": number, "status": "pending" }`
4. `team_task_update`
   - 参数: `{ "task_id": string, "expected_revision": number, "action": "claim" | "complete" | "release", "owner"?: string }`
   - 返回: `{ "task_id": string, "revision": number, "status": string }`
5. `team_task_list`
   - 参数: `{}`
   - 返回: `{ "tasks": Array<TeamTaskItem> }`

### 验收矩阵
- **AC-01**: `subagent` 独立上下文派生，隔离子会话并返回聚焦结果。
- **AC-02**: `workflow` 按照拓扑排序（DAG）依次执行无环步骤。
- **AC-03**: `team_task_*` 支持共享看板任务创建、CAS 乐观锁变更（claim/complete）及写域冲突隔离。
- **AC-04**: 防护递归深度爆炸（MAX_SUBAGENT_DEPTH ≤ 2）。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与红相验证
- **负责人**: Team Lead
