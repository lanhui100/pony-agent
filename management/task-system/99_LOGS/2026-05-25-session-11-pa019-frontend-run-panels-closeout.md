# 2026-05-25 Session 11 Closeout

## 本轮目标
- 收口前端 run 控制面 UI
- 收口前端附件中心 UI
- 完成 `PA-019` 最小 graph planner / policy
- 跑通前后端验证并回写任务系统 / 架构文档

## 本轮实现
- 前端右侧新增 `Graph Run` 控制面：
  - 支持 `start_graph_run / continue_graph_run / resume_graph_run / stop_graph_run`
  - 展示当前/最近 run、run phase、goal、session、更新时间、checkpoint 摘要
- 前端主工作区新增附件中心：
  - 展示附件资产列表、生命周期状态、引用计数、session / MIME / 名称筛选
  - 保留 cleanup 交互入口，并明确区分“UI 集成点”与“后端命令尚未暴露”
- 后端完成 `PA-019`：
  - 新增 `GraphPlanner / GraphPlanningContext / DefaultGraphPlanner`
  - graph run 现在会基于稳定 `GraphRun + GraphTurnHandoff` 输出 `continue / wait_user`
  - 当前 `continue` 只产生可审计决策，不自动递归开启下一轮 turn

## 关键边界
- runtime 继续只负责单 turn 内的 `model -> tool -> model` 收口
- graph planner 位于 graph 层，而不是 runtime turn preflight
- 宿主 UI 只消费 control plane / inspection 结果，不承载 graph/runtime 核心语义

## 验证
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- `npx vitest run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeSidebar.spec.ts`

## 产出文件
- `src-tauri/src/agent/planner.rs`
- `src-tauri/src/agent/graph.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/components/GraphRunControlPanel.vue`
- `src/components/AttachmentCenterPanel.vue`
- `src/components/HomeSidebar.vue`
- `src/components/HomeWorkspace.vue`
- `tests/HomeSidebar.spec.ts`
- `tests/HomeWorkspace.spec.ts`
