# 2026-06-05 PA-038 / PA-039 / PA-040 Spec Tightening Review

## 审核方式

- 独立子智能体对 `PA-038 / PA-039 / PA-040` 的任务卡、OpenSpec spec 与架构母文档做只读复核
- 主线程只采纳与“当前真实实现边界”和“关闭条件可验证性”直接相关的意见

## 采纳的核心意见

1. 当前不必新增第四张近线 hooks 大卡
   - `PA-038 / PA-039 / PA-040` 这一层拆分已经基本足够可执行
   - 只有当 `PA-040` 的 planner 与 capability 两条线持续互相阻塞时，才考虑拆出 `PA-040A`

2. `PA-038` 的 persisted evidence / read-plane 验收口径需要再收紧
   - 当前阶段最小必达 boundary 应明确收口到 `submission_plan / wait_user / stop_requested / run_resume`
   - 测试口径应从“大而全清单”改为这些 canonical boundary 的 roundtrip 断言

3. `PA-039` 需要显式区分“当前 memory-write 真边界”与“未来通用 side-effect 扩展”
   - 当前关闭条件应只要求 `long-term memory write` 路径完成 truth-source 与 recovery evidence 闭环
   - 通用 side-effect 扩展不应作为本卡关闭前置

4. `PA-040` 的 read-plane / monitor 验收表述过宽
   - 当前至少应要求 `planner preflight / tool selection / graph decision` 与 `capability resolve / skill mediation` evidence 进入 turn truth-source
   - `monitor` 若尚未完全落地，不应提前写成完成态口径

## 已执行的修订

- 更新 `PA-038` 任务卡与 OpenSpec spec：
  - 明确当前最小 persisted audit chain
  - 把测试口径收紧为 canonical boundary + reload/read-plane roundtrip
- 更新 `PA-039` 任务卡与 OpenSpec spec：
  - 显式限定当前范围为 `long-term memory write`
  - 不再把“通用 side-effect 扩展”写成关闭前置
- 更新 `PA-040` 任务卡与 OpenSpec spec：
  - 明确 planner/capability evidence 的最小读面
  - 把 `monitor` 从“必须全量 closeout”改成“若未落地，不作为本卡关闭前置”

## 结论

- 当前仍建议沿 `PA-038 -> PA-039 -> PA-040(graph decision 优先) -> PA-040(capability drilldown/reload)` 的顺序继续推进
- 在 `PA-040` 真正出现双线互卡之前，不建议新增额外 hooks 主任务卡
