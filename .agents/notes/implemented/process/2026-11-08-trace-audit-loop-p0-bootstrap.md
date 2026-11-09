# Agent Note: Trace 审核 Loop 批准落地（ADR 0101 迁 implemented + P0 地基 + r1-dim6 启动）

Status: implemented

## Context

用户批准 trace 审核 loop 设计，并修正触发形态：**不是一次性任务、不必是定时器，由用户唤起、中期持续**。
原 proposed ADR 0101 以 cron + headless tick 为选中方案，需按批准反馈修订后采纳。

## Decision

1. ADR 0101 修订为「用户唤起 + 持久状态机 + 确定性骨架 + 单一审批门」，迁主目录 `Status: implemented`
   （README 正典：implemented 描述决定采纳，分阶段进度归任务系统）；定时器降级为可选演进不列验收。
2. 中期性由双模式保证：queue（6→1/3→2→5→7→4）走完转 patrol（每次唤起棘轮全集 + corpus 重放，回归开新轮）。
3. 补 scratch worktree 隔离条款：replay 双臂在一次性 worktree 固定提交执行，不污染主仓。
4. 任务卡 PA-168/169/170 串行登记；P0 物理落地：`docs/audit/loop-state.json`（phase=collect）、
   corpus 3 只读任务 + manifest（aggregate sha256 879597eb…）、skill 唤起入口、人读 README。
5. r1-dim6 首条 collect 回执：check-langfuse.sh exit=3（UNKNOWN）——Tempo 语义轨候选 115 条，
   但审计环境缺 LANGFUSE 凭据；凭据供给列为首个审计发现项，UNKNOWN ≠ PASS。
6. observability.yaml 补 loop 自举归属注释（复用既有 OTel 出口，无新遥测面）。

## Alternatives considered

- **定时器无人值守（原选中）**：批准时否决为核心形态——节奏应由人掌控，无人值守放大假 PASS/未批准实施风险；保留为可选演进。
- **create_goal 会话内续行**：pause/resume 需人类意图、长会话上下文膨胀，跨会话仍靠状态文件——落选。
- **常驻 dev team**：team 状态会话内即失忆，与跨 tick 状态文件架构冲突，多 writer 破坏写域冻结——不采用。
- **纯确定性驱动直调 LLM**：需重造 harness 工具循环/治理/trace 出口，违反复用——保留为远期演进。
- 完整比较记录见 ADR 0101 候选方案节（本篇不重复事实，仅记落地取舍）。

## Receipts

- `docs/audit/rounds/r1-dim6/receipts/001-collect-langfuse.{json,meta.json}`：exit=3，verdict=unknown。
- corpus hash 对拍：`sha256sum` 逐任务 + 拼接聚合与 manifest 一致（见验证命令输出）。
- ADR Status 门禁：`grep -L "^Status:"` 与主目录纯 implemented 检查期望为空。