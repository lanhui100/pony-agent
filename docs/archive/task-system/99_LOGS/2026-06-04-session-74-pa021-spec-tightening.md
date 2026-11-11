# 2026-06-04 Session 74

## 主题

- 收紧 `PA-021` 的 spec 边界
- 把 skills registry / bridge 从原则性定义推进到可直接开工的 `v1` 实现切片

## 本轮完成

1. 复核 `PA-021` 任务卡、OpenSpec proposal/design/spec/tasks 与当前代码现状
2. 并行使用只读子智能体审查：
   - 当前代码里最自然的接入点
   - `PA-021` spec 是否足够进入实现
3. 已形成并采纳的关键结论：
   - `v1` 先只执行 `tool`-composed skills
   - `resource / prompt_template` 组合在 `v1` 先进入 registry / inspect / observability，不要求可执行
   - 第一刀应落在 `control_plane -> snapshot ingress -> capability registry` 适配层
   - 不应先改 planner / graph 主循环，也不应新开前端第二套 skills store
   - 权限、失败、观测口径需要在 spec 里先补结构化要求
4. 已新增独立 spec review：
   - [2026-06-04-pa021-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa021-spec-review.md)
5. 已回写并收紧：
   - [PA-021 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-021-build-skills-registry-and-bridge.md)
   - [skills-registry-bridge/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge/specs/skills-registry-bridge/spec.md)
   - [skills-registry-bridge/design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge/design.md)
   - [skills-registry-bridge/tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge/tasks.md)

## 当前判断

- `PA-021` 不再适合继续停留在“泛 spec-first”状态
- 现在已经具备进入第一段实现的文档前置条件
- 最小实现顺序应为：
  - registry model
  - source snapshot ingress adapter
  - `list_skills / inspect_skill`
  - tool-only runtime resolution
  - observability
  - planner 最小消费
