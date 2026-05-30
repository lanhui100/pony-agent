# PA-024 构建模型监控面与 telemetry 聚合边界

## 状态
- Status: `Backlog`
- Priority: `P2`
- Owner: `Codex`

## 目标
把当前已经出现在导航与页面层的 `ModelMonitorPage` 从占位骨架升级为正式能力面，建立最小可扩展的 telemetry 聚合边界，让 provider / model / tool 协同的运行指标能够被稳定读取、展示与后续审计。

## 输出
- telemetry 聚合读面第一版：明确哪些指标来自 turn trace、哪些来自 provider、哪些来自 run/session 聚合
- 模型监控页第一版真实数据面：至少覆盖调用量、成功率、首 token 延迟或总耗时、失败类型/降级信息
- provider / model 维度的最小筛选与摘要卡片
- 模型监控与 `HostControlPlane` / inspection / telemetry 的边界说明
- 对应前端与 Rust 测试补齐

## 验收标准
- `ModelMonitorPage` 不再只是占位文案，而是能展示真实指标
- UI 读取的是稳定聚合结果，而不是直接拼接 runtime 内部瞬时状态
- 指标边界清晰区分 `turn trace`、`graph run`、`provider health` 与未来长期趋势
- 前端导航、数据读取与回归测试保持可用
- 文档明确本卡第一版不要求完整告警系统、预算管理或跨设备汇总平台

## 当前进展
- `src/App.vue` 已把 `model-monitor` 纳入页面路由分支
- `src/components/HomeSessionSidebar.vue` 已暴露“模型监控”导航入口
- `src/components/ModelMonitorPage.vue` 已有页面骨架与扩展方向文案
- `tests/HomeSessionSidebar.spec.ts` 已覆盖导航切换行为
- 当前 telemetry 仍主要停留在 turn trace、tool activity、provider 回执和本地 inspection 层，尚未形成专门聚合面

## 下一步动作
- 先定义最小监控指标集合与聚合来源
- 再决定是扩展 `inspect_host`，还是新增独立 telemetry 读取命令
- 最后把页面骨架接入真实数据，并补齐前后端回归测试

## 当前卡点
- 如果在 `PA-018` 的 context/state retrieval 边界未稳定前直接扩写监控面，容易把 telemetry、session summary、run state 与 planner 审计字段再次耦合

## 断点续跑提示
继续前先看：
- `src/components/ModelMonitorPage.vue`
- `src/components/HomeSessionSidebar.vue`
- `src/stores/runtime.ts`
- `src-tauri/src/agent/control_plane.rs`
- `src-tauri/src/agent/telemetry.rs`
