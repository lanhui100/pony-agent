# Design: Add Session Control Surface And Feedback Loop

## Context

当前“session 可控”存在一个典型的层间断裂：

- runtime / control-plane 已具备控制能力
- runtime store 已能仲裁 start / resume / continue / replay
- session sidebar 已能消费一部分 history graph
- 但用户还看不到完整、稳定、可解释的 session 控制面

这会导致一个假完成态：

- 工程上能停、能恢复、能 checkout
- 但用户不知道自己当前是 `live / historical / historical_dirty / paused / recovery-capable`
- 也不知道某次 restore 是否只恢复了 transcript

## Design Decisions

### 1. 本卡只消费既有后端合同

- 不新增核心控制命令
- 不重写 `submissionPlan`、`recoveryMode`、`HistoryRestoreResult`
- 不改 `PA-028 / PA-032 / PA-034` 的核心语义

### 2. session 控制入口要显式化，而不是“隐式靠下一次发送”

- 若当前存在 paused run 或 recovery-capable checkpoint
- 前端必须给出明确 CTA，例如：
  - `继续`
  - `恢复`
  - `重新开始`
- 而不是让用户只能靠再发一条消息去隐式触发 store 仲裁

### 3. degrade result 必须可见

- 对用户来说，`transcript_only` 与 `transcript_and_workspace` 的差别是关键事实
- 若 checkout / restore 降级，必须给出：
  - 是否恢复了工作区
  - 如果没有，原因是什么

### 4. 历史态与运行态要共享一套状态语言

- `live`
- `historical`
- `historical_dirty`
- `paused`
- `recovery_capable`

这些状态应投影为统一前端文案和按钮可用性，而不是只显示原始 flag。

## Implementation Sketch

1. 在 `src/stores/runtime.ts` 收紧 session control CTA 所需的派生状态
2. 在 `src/components/HomeSessionSidebar.vue` 暴露显式的 session control panel
3. 在 `src/components/HomeWorkspace.vue` 暴露 stop / resume / replay 反馈
4. 在前端测试中覆盖 degrade result、disabled reason、mode 文案与 CTA 派发

## Verification

- `HomeSessionSidebar.spec.ts`
- `HomeWorkspace.spec.ts`
- `runtime-store.spec.ts`

验证重点：

- stop CTA 能命中既有 `stopTurn()`
- resume/replay CTA 能消费既有 `submissionPlan`
- degrade result 与 historical_dirty 能被显式展示
- mode 切换与按钮禁用原因一致

