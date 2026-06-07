# PA-021 建立 skills registry 与 bridge

## 状态
- Status: `Done`
- Priority: `P3`
- Owner: `Codex`

## OpenSpec Change
- [add-skills-registry-bridge](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge)

## Delta Spec
- [skills-registry-bridge/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge/specs/skills-registry-bridge/spec.md)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
在 planner、memory 与 MCP capability bridge 具备最小稳定边界后，引入 skills 体系作为可组合的能力封装层，让 skills 成为 planner 可选择、graph 可审计、runtime 可执行的高层能力单元，而不是零散 prompt 模板或宿主私有脚本。

## 输出
- skills registry 第一版模型
- skill manifest / capability fact / invocation boundary
- skills 与 tools / MCP / memory retrieval 的组合规则
- planner 消费 skills 的最小策略
- skills 可观测性与审计字段

## 验收标准
- skills 被定义为能力封装层，而不是 turn runtime 或宿主控制层
- planner 能读取 skill capability facts，但不依赖 skill 的实现细节
- skill invocation 与普通 tool / MCP invocation 的边界清楚
- skill 不直接持有宿主私有状态，而是通过统一 control plane / capability bridge 接入
- 文档明确本卡不要求完整 marketplace、分发平台或复杂沙箱

## 当前进展
- 当前项目已经有“skills”概念来源，但 Pony Agent 本体还没有正式的 skills registry
- `PA-020` 预计先把 MCP 能力接入层抽象出来
- `PA-019` 预计先建立 planner 的上层决策消费面
- 已补 `add-skills-registry-bridge` 的 OpenSpec `proposal / design / tasks / delta spec`，当前进入 spec-first 收口与实现拆分阶段
- 已完成一轮独立 spec 审核并采纳修订，见：
  [2026-06-04-pa021-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa021-spec-review.md)
- 已收紧 `v1` 实现边界：
  - `v1` 先只执行 `tool`-composed skills
  - `resource / prompt_template` 组合先进入 registry / inspect / observability，不要求本轮可执行
  - 第一刀先落在 `control_plane -> snapshot ingress -> capability registry` 适配层，而不是先改 planner / graph 主循环
  - 权限、失败、观测口径已补最小结构化要求，避免 skill 重新发明一套宿主私有语义
- 已完成第一段实现闭环：
  - Rust `SkillSourceSnapshot / SkillDescriptor / SkillFailureLayer` 已落地
  - `HostControlPlane.apply_skill_source_snapshot()` 已支持先归一化后原子同步到 runtime / control-plane registry
  - 已补 `list_skills / inspect_skill` 最小读面与 Tauri 命令入口
  - runtime 已支持 `tool`-only skill resolution / execution，并对非 `tool` 组合显式返回 unsupported
  - capability telemetry 已补 skill lineage 字段：`skillId / skillSourceId / composedCapabilityRefs / composedCapabilityKinds / failureLayer`
- 已完成第一批定向验证，见：
  [2026-06-04-pa021-implementation-slice-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa021-implementation-slice-review.md)
- 已补齐剩余 `5.1 / 5.3`：
  - `planner_skills` 已进入 `build_request()` semistable context，只暴露 normalized skill facts
  - runtime 已支持按已注册 skill 名称执行 executable skill
  - monitor summary/drilldown 已补 skill selection / source / failure layer 聚合与 trace 内 skill lineage 展示
- 已形成验收审计：
  [2026-06-04-pa021-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa021-acceptance-audit.md)
- 已完成最终 closeout：
  - 本卡已按完成态口径从 `Review` 关闭到 `Done`
  - `v1` 范围不再继续吸收更高阶 skill selection policy、mixed composition execution 或宿主刷新链路扩展

## 下一步动作
- 后续若继续扩 skills，应拆到更高阶 planner selection、mixed composition execution 或宿主刷新链路任务，而不是继续扩大 `PA-021`
- 如需归档对应 OpenSpec change，再按变更归档流程单独执行

## 当前卡点
- 暂无；本卡已完成关闭

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-019-build-graph-planner-and-decision-policy.md`
- `management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md`
- `docs/architecture/overview.md`
