# PA-109: 长程自驱目标工具族 (goal_*)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具清单
1. `create_goal`
   - 参数: `{ "objective": string, "max_goal_rounds"?: number }`
   - 返回: `{ "goal": { "id": string, "objective": string, "phase": "active", "revision": number, "roundsStarted": number, "maxGoalRounds": number } }`
2. `get_goal`
   - 参数: `{}`
   - 返回: `{ "goal": Option<GoalData> }`
3. `update_goal`
   - 参数: `{ "goal_id": string, "revision": number, "action": "complete" | "pause" | "resume" | "edit" | "blocked", "objective"?: string, "blocked_reason"?: string, "max_goal_rounds"?: number }`
   - 返回: `{ "goal": GoalData }`

### 验收矩阵
- **AC-01**: `create_goal` 成功持久化目标实体，设定轮数预算与初始版本号 `revision = 1`。
- **AC-02**: `get_goal` 幂等获取当前会话目标及其最新状态。
- **AC-03**: `update_goal` 基于 CAS 机制校验 `revision`，实现状态迁移（active -> paused -> resumed -> completed / blocked），版本号递增。
- **AC-04**: 连续受阻保护：当处于 `blocked` 状态时，必须提供结构化 `blocked_reason`，防无休止空转。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与红相验证
- **负责人**: Team Lead
