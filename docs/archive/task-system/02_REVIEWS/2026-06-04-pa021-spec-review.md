# PA-021 Spec Review

## 审核范围

- `management/task-system/03_TASKS/PA-021-build-skills-registry-and-bridge.md`
- `openspec/changes/add-skills-registry-bridge/proposal.md`
- `openspec/changes/add-skills-registry-bridge/design.md`
- `openspec/changes/add-skills-registry-bridge/specs/skills-registry-bridge/spec.md`
- `openspec/changes/add-skills-registry-bridge/tasks.md`

## 审核结论

总体方向正确，但当前文档仍偏“原则定义”，还不够无歧义地直接全面开工。结论是：

- 可以进入 `v1` 小步实现
- 但应先补一轮 spec 收口，把第一段实现边界钉死

## 采纳修改

1. 明确 `v1` runtime execution scope
   - `v1` 先只执行 `tool`-composed skills
   - `resource / prompt_template` 组合在 `v1` 先进入 registry / inspect / observability，不要求可执行
2. 明确 skills 的第一段接入点
   - 先走 `control_plane -> snapshot ingress -> capability registry` 适配层
   - 复用现有 unified capability registry，而不是先改 planner / graph / runtime 主循环
3. 明确权限聚合口径
   - 只要任一底层 capability `requiresApproval=true`，skill 即视为 `requiresApproval=true`
   - 只要任一底层 capability `hostMediated=true`，skill 即视为 `hostMediated=true`
   - `permissionScope` 需要提供聚合后的可读摘要，不能丢失底层能力范围事实
4. 明确失败与观测最小字段
   - `v1` 至少区分：skill resolution failure、source unavailability、permission denial、malformed composition、underlying capability execution failure
   - 观测至少暴露：`skillId / sourceId / composedCapabilityRefs / composedCapabilityKinds / failureLayer`
5. 明确第一段实现顺序
   - registry model
   - source snapshot ingress adapter
   - host/control-plane read surfaces
   - tool-only runtime resolution
   - observability
   - planner 最小消费

## 结果

- 上述意见已采纳并回写到 `PA-021` 的任务卡、design、spec、tasks
- `PA-021` 现在可以进入第一段实现，而不是继续停留在泛泛的 spec-first 状态
